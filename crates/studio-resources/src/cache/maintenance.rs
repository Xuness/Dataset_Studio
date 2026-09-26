use super::*;

impl PreviewCache {
    pub fn set_quota(&self, bytes: u64) -> Result<CacheMetrics> {
        if bytes > 1024 * 1024 * 1024 * 1024 {
            return Err(Error::invalid("缓存配额最多 1 TiB"));
        }
        let _maintenance = self.maintenance.lock().map_err(cache_error)?;
        let (settings, clear) = {
            let state = self.inner.lock().map_err(cache_error)?;
            (state.settings.clone(), state.metrics.clear_pending)
        };
        save_settings(&settings, bytes, clear)?;
        {
            let mut state = self.inner.lock().map_err(cache_error)?;
            state
                .db
                .execute(
                    "UPDATE settings SET value=?1 WHERE key='quota_bytes'",
                    [bytes as i64],
                )
                .map_err(cache_error)?;
            state.metrics.quota_bytes = bytes;
            state.update_pending();
        }
        self.maintain_locked(256)?;
        Ok(self.metrics())
    }

    /// Start durable, bounded clearing. In-flight writes are discarded, active
    /// readers stay pinned, and the maintenance loop continues after they finish.
    pub fn clear(&self) -> Result<CacheMetrics> {
        let _maintenance = self.maintenance.lock().map_err(cache_error)?;
        let (settings, objects, quota) = {
            let state = self.inner.lock().map_err(cache_error)?;
            (
                state.settings.clone(),
                state.objects.clone(),
                state.metrics.quota_bytes,
            )
        };
        ensure_objects(&objects)?;
        save_settings(&settings, quota, true)?;
        let scan = fs::read_dir(objects).map_err(cache_error)?;
        {
            let mut state = self.inner.lock().map_err(cache_error)?;
            state.metrics.clear_pending = true;
            state.scan = Some(scan);
            state.update_pending();
        }
        self.maintain_locked(128)?;
        Ok(self.metrics())
    }

    pub fn maintain(&self, limit: usize) -> Result<()> {
        let _maintenance = match self.maintenance.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(()),
            Err(error) => return Err(cache_error(error)),
        };
        self.maintain_locked(limit.min(256))
    }

    fn maintain_locked(&self, limit: usize) -> Result<()> {
        let started = Instant::now();
        let keys = {
            let state = self.inner.lock().map_err(cache_error)?;
            if state.metrics.clear_pending || state.metrics.bytes > state.metrics.quota_bytes {
                state
                    .db
                    .prepare("SELECT key FROM entries ORDER BY used_ms,key LIMIT ?1")
                    .map_err(cache_error)?
                    .query_map(
                        [(limit + state.pins.len() + state.busy.len()) as i64],
                        |r| r.get::<_, String>(0),
                    )
                    .map_err(cache_error)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(cache_error)?
            } else {
                Vec::new()
            }
        };
        let mut removed = 0;
        for key in keys {
            {
                let state = self.inner.lock().map_err(cache_error)?;
                if removed >= limit
                    || (!state.metrics.clear_pending
                        && state.metrics.bytes <= state.metrics.quota_bytes)
                {
                    break;
                }
            }
            if self.remove_entry(&key)? {
                self.inner.lock().map_err(cache_error)?.metrics.evicted += 1;
                removed += 1;
            }
        }
        for _ in 0..limit {
            let next = {
                let mut state = self.inner.lock().map_err(cache_error)?;
                let next = state.scan.as_mut().and_then(Iterator::next);
                if next.is_none() {
                    state.scan = None;
                }
                next
            };
            let Some(entry) = next else { break };
            let entry = entry.map_err(cache_error)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let (orphan, reservation) = {
                let mut state = self.inner.lock().map_err(cache_error)?;
                if let Some(key) = name.strip_suffix(".jpg").filter(|k| valid_key(k)) {
                    if state.pins.contains_key(key) || state.busy.contains(key) {
                        continue;
                    }
                    let indexed: bool = state
                        .db
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM entries WHERE key=?1)",
                            [key],
                            |r| r.get(0),
                        )
                        .map_err(cache_error)?;
                    if indexed {
                        continue;
                    }
                    state.busy.insert(key.into());
                    (
                        true,
                        Some(Mutation {
                            inner: self.inner.clone(),
                            key: key.into(),
                            partial: None,
                            bytes: 0,
                        }),
                    )
                } else {
                    (
                        name.starts_with("partial-") && !state.partials.contains(&path),
                        None,
                    )
                }
            };
            if orphan {
                let objects = path
                    .parent()
                    .ok_or_else(|| Error::new("CACHE_PATH_INVALID", "缓存材料目录无效"))?;
                ensure_objects(objects)?;
                let kind = match entry.file_type() {
                    Ok(kind) => kind,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(cache_error(e)),
                };
                if kind.is_file() {
                    match fs::remove_file(&path) {
                        Ok(()) => {
                            self.inner
                                .lock()
                                .map_err(cache_error)?
                                .metrics
                                .maintenance_removed += 1
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(cache_error(e)),
                    }
                }
            }
            drop(reservation);
        }
        self.finish_clear()?;
        self.inner.lock().map_err(cache_error)?.update_pending();
        tracing::debug!(target: "studio_resources::cache", operation = "maintain", removed,
            elapsed_us = started.elapsed().as_micros() as u64, "preview cache maintenance");
        Ok(())
    }

    pub(super) fn remove_entry(&self, key: &str) -> Result<bool> {
        self.remove_with(key, || {})
    }

    pub(super) fn invalidate(
        &self,
        key: &str,
        size: u64,
        path: PathBuf,
        pin: CachePin,
    ) -> Result<()> {
        let reservation = {
            let mut state = self.inner.lock().map_err(cache_error)?;
            if state.pins.get(key) == Some(&1) {
                // Transfer protection before releasing our read pin. Otherwise a
                // new writer could repair the entry in between and be deleted.
                state.busy.insert(key.into());
                Some(Mutation {
                    inner: self.inner.clone(),
                    key: key.into(),
                    partial: None,
                    bytes: 0,
                })
            } else {
                None
            }
        };
        drop(pin);
        if let Some(reservation) = reservation {
            self.remove_reserved(path, size, reservation, || {})?;
        }
        Ok(())
    }
    pub(super) fn remove_with(&self, key: &str, before_io: impl FnOnce()) -> Result<bool> {
        if !valid_key(key) {
            return Err(Error::new("CACHE_PATH_INVALID", "缓存索引身份无效"));
        }
        let (path, size, reservation) = {
            let mut state = self.inner.lock().map_err(cache_error)?;
            if state.pins.contains_key(key) || state.busy.contains(key) {
                return Ok(false);
            }
            let size: Option<u64> = state
                .db
                .query_row("SELECT bytes FROM entries WHERE key=?1", [key], |r| {
                    unsigned(r, 0)
                })
                .optional()
                .map_err(cache_error)?;
            let Some(size) = size else {
                return Ok(false);
            };
            state.busy.insert(key.into());
            (
                state.file(key),
                size,
                Mutation {
                    inner: self.inner.clone(),
                    key: key.into(),
                    partial: None,
                    bytes: 0,
                },
            )
        };
        self.remove_reserved(path, size, reservation, before_io)?;
        Ok(true)
    }

    fn remove_reserved(
        &self,
        path: PathBuf,
        size: u64,
        reservation: Mutation,
        before_io: impl FnOnce(),
    ) -> Result<()> {
        before_io();
        let objects = path
            .parent()
            .ok_or_else(|| Error::new("CACHE_PATH_INVALID", "缓存材料目录无效"))?;
        ensure_objects(objects)?;
        // Unlink a validated fixed child, never traverse its target. The key
        // reservation prevents open/replace while its old file is being removed.
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(cache_error(e)),
        }
        {
            let mut state = self.inner.lock().map_err(cache_error)?;
            state
                .db
                .execute("DELETE FROM entries WHERE key=?1", [&reservation.key])
                .map_err(cache_error)?;
            state.metrics.entries -= 1;
            state.metrics.bytes -= size;
        }
        drop(reservation);
        Ok(())
    }

    // All configuration changes use the maintenance gate, but file fsync never
    // holds the directory-state mutex used by ordinary cache hits.
    fn finish_clear(&self) -> Result<()> {
        let completed = {
            let state = self.inner.lock().map_err(cache_error)?;
            (state.metrics.clear_pending
                && state.metrics.entries == 0
                && state.scan.is_none()
                && state.busy.is_empty())
            .then(|| (state.settings.clone(), state.metrics.quota_bytes))
        };
        if let Some((settings, quota)) = completed {
            save_settings(&settings, quota, false)?;
            self.inner
                .lock()
                .map_err(cache_error)?
                .metrics
                .clear_pending = false;
        }
        Ok(())
    }
}

impl State {
    pub(super) fn file(&self, key: &str) -> PathBuf {
        self.objects.join(format!("{key}.jpg"))
    }
    pub(super) fn update_pending(&mut self) {
        self.metrics.maintenance_pending = self.scan.is_some()
            || self.metrics.clear_pending
            || self.metrics.bytes > self.metrics.quota_bytes;
    }
}
