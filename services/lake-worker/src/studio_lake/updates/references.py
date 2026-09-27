"""Source tag dictionaries with archived responses and explicit observation context."""

import json
import time

from ..library import Batch
from ..util import digest, stable_id, now
from .archive import io_lock
from .sites import UpdateError


def archive(state, lib, job, response, resource):
    key = stable_id("update-reference-v1", job["id"], resource, digest(response.body))
    with io_lock(state.root, lib), lib.writer_lock():
        old = lib.committed_key(key)
        if old:
            return old["batch_id"]
        batch = Batch(
            lib,
            key,
            {
                "kind": "update_reference",
                "update_job_id": job["id"],
                "update_role": "reference",
                "definition": job["definition"],
                "resource": resource,
                "observed_at": now(),
                "request": {**response.request, "status": response.status},
                "response_sha256": digest(response.body),
            },
        )
        batch.write_bytes("response_body.bin", response.body)
        batch.commit()
        return batch.id


def categories(state, lib, job, site, rows, selected, cancelled):
    if site.name == "danbooru" or not selected:
        return None
    names = {
        tag
        for _, record in rows
        if record["id"] in selected
        for tag in str(record.get("tags") or "").split(" ")
        if tag
    }
    if len(names) > 50000:
        raise UpdateError("UPDATE_RESOURCE_LIMIT", "Tag dictionary page exceeds 50000 names")
    with state.db() as db:
        db.execute(
            "CREATE TABLE IF NOT EXISTS tag_types(lake_id TEXT NOT NULL,name TEXT NOT NULL,category INTEGER,observed REAL NOT NULL,batch_id TEXT NOT NULL,PRIMARY KEY(lake_id,name))"
        )
        db.execute(
            "CREATE TABLE IF NOT EXISTS tag_summary(lake_id TEXT PRIMARY KEY,version INTEGER,observed REAL NOT NULL)"
        )
        cached = {}
        for name in names:
            found = db.execute(
                "SELECT category,observed FROM tag_types WHERE lake_id=? AND name=?", (job["lake_id"], name)
            ).fetchone()
            if found and found[1] > time.time() - 7 * 86400:
                cached[name] = found[0]
    needed = names - set(cached)

    def fetch(params, resource):
        response = site.request(params, cancelled, resource=resource)
        batch_id = archive(state, lib, job, response, resource)
        if response.status != 200:
            raise UpdateError("UPDATE_REMOTE_ERROR", f"Tag API HTTP {response.status}", response.retry_after)
        try:
            data = json.loads(response.body)
            if isinstance(data, str):
                data = json.loads(data)
            return data, batch_id
        except (ValueError, TypeError):
            raise UpdateError(
                "UPDATE_RESPONSE_INVALID", "Tag API schema invalid; response retained"
            ) from None

    def save(records, batch_id):
        pending = []
        for name, category in records:
            if not isinstance(name, str) or not isinstance(category, int) or isinstance(category, bool):
                raise UpdateError("UPDATE_RESPONSE_INVALID", "Invalid tag category")
            pending.append((job["lake_id"], name, category, time.time(), batch_id))
            if name in names:
                cached[name] = category
            if len(pending) == 2048:
                with state.db() as db:
                    db.executemany("INSERT OR REPLACE INTO tag_types VALUES(?,?,?,?,?)", pending)
                pending = []
        if pending:
            with state.db() as db:
                db.executemany("INSERT OR REPLACE INTO tag_types VALUES(?,?,?,?,?)", pending)

    if needed and site.name == "yandere":
        with state.db() as db:
            summary = db.execute(
                "SELECT version,observed FROM tag_summary WHERE lake_id=?", (job["lake_id"],)
            ).fetchone()
        if not summary or summary[1] < time.time() - 3600:
            data, batch_id = fetch({"version": summary[0]} if summary else {}, "tag_summary")
            if not isinstance(data, dict) or not isinstance(data.get("version"), int):
                raise UpdateError("UPDATE_RESPONSE_INVALID", "Tag summary version is invalid")
            if not data.get("unchanged"):
                text = data.get("data")
                if not isinstance(text, str):
                    raise UpdateError("UPDATE_RESPONSE_INVALID", "Tag summary data is invalid")

                def entries():
                    for value in text.split(" "):
                        if value:
                            fields = value.split("`")
                            if len(fields) < 3 or not fields[0].isdigit():
                                raise UpdateError("UPDATE_RESPONSE_INVALID", "Tag summary entry is invalid")
                            yield fields[1], int(fields[0])

                save(entries(), batch_id)
            with state.db() as db:
                db.execute(
                    "INSERT OR REPLACE INTO tag_summary VALUES(?,?,?)",
                    (job["lake_id"], data["version"], time.time()),
                )
    needed = sorted(names - set(cached))
    width = 50 if site.name == "gelbooru" else 1
    for offset in range(0, len(needed), width):
        group = needed[offset : offset + width]
        params = (
            {"name": group[0] + "*", "limit": 100, "order": "name"}
            if site.name == "yandere"
            else {"page": "dapi", "s": "tag", "q": "index", "json": 1, "limit": 100, "names": " ".join(group)}
        )
        data, batch_id = fetch(params, "tags")
        records = data.get("tag", []) if isinstance(data, dict) else data
        if not isinstance(records, list):
            raise UpdateError("UPDATE_RESPONSE_INVALID", "Tag list schema is invalid")
        save(((r.get("name"), r.get("type")) for r in records), batch_id)
        # Missing categories remain unknown rather than becoming general tags.
        with state.db() as db:
            for name in set(group) - set(cached):
                db.execute(
                    "INSERT OR REPLACE INTO tag_types VALUES(?,?,NULL,?,?)",
                    (job["lake_id"], name, time.time(), batch_id),
                )
                cached[name] = None
    return {name: cached.get(name) for name in names}
