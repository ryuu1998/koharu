use std::time::Duration;

use tokio::sync::watch;

#[derive(Clone, Copy, Default)]
struct State {
    stopped: bool,
    paused: bool,
    active: usize,
}

/// Admission and draining shared by model jobs and remote request attempts.
#[derive(Clone)]
pub struct RunControl(watch::Sender<State>);

impl Default for RunControl {
    fn default() -> Self {
        Self(watch::channel(State::default()).0)
    }
}

impl RunControl {
    pub fn stop(&self) {
        self.0.send_modify(|state| state.stopped = true);
    }
    pub fn pause(&self) {
        self.0.send_modify(|state| state.paused = true);
    }
    pub fn resume(&self) {
        self.0.send_modify(|state| state.paused = false);
    }
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.0.borrow().stopped
    }
    #[must_use]
    pub fn paused(&self) -> bool {
        self.0.borrow().paused
    }

    pub async fn ready(&self) -> bool {
        let mut state = self.0.subscribe();
        loop {
            let current = *state.borrow_and_update();
            if current.stopped {
                return false;
            }
            if !current.paused {
                return true;
            }
            if state.changed().await.is_err() {
                return false;
            }
        }
    }

    pub async fn drained(&self) -> bool {
        let mut state = self.0.subscribe();
        loop {
            let current = *state.borrow_and_update();
            if current.stopped || !current.paused {
                return false;
            }
            if current.active == 0 {
                return true;
            }
            if state.changed().await.is_err() {
                return false;
            }
        }
    }

    pub async fn admit(&self) -> Option<Activity> {
        loop {
            if !self.ready().await {
                return None;
            }
            let mut admitted = false;
            self.0.send_if_modified(|state| {
                if state.stopped || state.paused {
                    return false;
                }
                state.active += 1;
                admitted = true;
                true
            });
            if admitted {
                return Some(Activity(self.clone()));
            }
        }
    }

    /// Stop wakes admission and retry waits without dropping an in-flight model job.
    pub async fn cancelled(&self) {
        let mut state = self.0.subscribe();
        loop {
            if state.borrow_and_update().stopped {
                return;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    }

    pub async fn delay(&self, duration: Duration) -> bool {
        tokio::select! {
            () = self.cancelled() => false,
            () = tokio::time::sleep(duration) => self.ready().await,
        }
    }
}

pub struct Activity(RunControl);
impl Drop for Activity {
    fn drop(&mut self) {
        self.0.0.send_modify(|state| state.active -= 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pause_drains_then_resume_reopens_admission() {
        let control = RunControl::default();
        let active = control.admit().await.unwrap();
        control.pause();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), control.drained())
                .await
                .is_err()
        );
        drop(active);
        assert!(control.drained().await);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), control.admit())
                .await
                .is_err()
        );
        control.resume();
        assert!(control.admit().await.is_some());
    }

    #[tokio::test]
    async fn stop_wakes_paused_admission_and_long_retry_delays() {
        let control = RunControl::default();
        control.pause();
        control.stop();
        assert!(control.admit().await.is_none());
        assert!(!control.delay(Duration::from_secs(600)).await);
    }
}
