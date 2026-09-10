use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// Lease time advances only while clients can acquire the exclusive cache lock.
pub(super) struct LeaseClock {
    state: Mutex<State>,
    #[cfg(test)]
    wall_time: Mutex<Option<Instant>>,
}

struct State {
    elapsed: Duration,
    resumed_at: Instant,
    paused: bool,
}

impl State {
    fn elapsed_at(&self, now: Instant) -> Duration {
        if self.paused {
            self.elapsed
        } else {
            self.elapsed + now.saturating_duration_since(self.resumed_at)
        }
    }
}

impl LeaseClock {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                elapsed: Duration::ZERO,
                resumed_at: Instant::now(),
                paused: false,
            }),
            #[cfg(test)]
            wall_time: Mutex::new(None),
        }
    }

    fn wall_now(&self) -> Instant {
        #[cfg(test)]
        if let Some(now) = *self.wall_time.lock().unwrap() {
            return now;
        }
        Instant::now()
    }

    pub(super) fn now(&self) -> Duration {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .elapsed_at(self.wall_now())
    }

    // The caller holds the cache lock, so pauses cannot overlap.
    pub(super) fn pause(&self) -> Pause<'_> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.elapsed = state.elapsed_at(self.wall_now());
        state.paused = true;
        Pause(self)
    }

    #[cfg(test)]
    pub(super) fn advance(&self, duration: Duration) {
        let mut wall_time = self.wall_time.lock().unwrap();
        *wall_time = Some(wall_time.unwrap_or_else(Instant::now) + duration);
    }
}

pub(super) struct Pause<'a>(&'a LeaseClock);

impl Drop for Pause<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.resumed_at = self.0.wall_now();
        state.paused = false;
    }
}
