use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use koharu_runtime::RunControl;
use tokio::{sync::watch, time::Instant};

#[derive(Default)]
pub(crate) struct Scheduler(Mutex<HashMap<&'static str, Arc<Admission>>>);

impl Scheduler {
    pub(crate) fn context(
        &self,
        provider: &'static str,
        config: &super::RemoteConfig,
        control: &RunControl,
    ) -> super::transport::RequestContext {
        let mut providers = self.0.lock().expect("remote admission lock poisoned");
        let admission = providers
            .entry(provider)
            .or_insert_with(|| Arc::new(Admission::new(config.parallelism())))
            .clone();
        admission.configure(config.parallelism());
        super::transport::RequestContext {
            control: control.clone(),
            admission,
        }
    }
}

#[derive(Clone)]
struct State {
    active: usize,
    limit: usize,
    ceiling: usize,
    successes: usize,
    not_before: Instant,
}

pub(super) struct Admission(watch::Sender<State>);

impl Admission {
    pub(super) fn new(ceiling: usize) -> Self {
        Self(
            watch::channel(State {
                active: 0,
                limit: ceiling,
                ceiling,
                successes: 0,
                not_before: Instant::now(),
            })
            .0,
        )
    }

    pub(super) fn configure(&self, ceiling: usize) {
        self.0.send_if_modified(|state| {
            if state.ceiling == ceiling {
                return false;
            }
            state.ceiling = ceiling;
            state.limit = state.limit.min(ceiling);
            true
        });
    }

    pub(super) async fn acquire(self: &Arc<Self>, control: &RunControl) -> Option<Slot> {
        let mut updates = self.0.subscribe();
        loop {
            if !control.ready().await {
                return None;
            }
            let mut admitted = false;
            self.0.send_if_modified(|state| {
                if state.active >= state.limit || Instant::now() < state.not_before {
                    return false;
                }
                state.active += 1;
                admitted = true;
                true
            });
            if admitted {
                return Some(Slot(self.clone()));
            }
            let next = {
                let state = updates.borrow_and_update();
                (state.not_before > Instant::now()).then_some(state.not_before)
            };
            // Recheck capacity after subscribing; permit release cannot be lost.
            let available = {
                let state = self.0.borrow();
                state.active < state.limit && Instant::now() >= state.not_before
            };
            if available {
                continue;
            }
            tokio::select! {
                () = control.cancelled() => return None,
                result = updates.changed() => { if result.is_err() { return None; } },
                () = async { match next { Some(next) => tokio::time::sleep_until(next).await, None => std::future::pending().await } } => {},
            }
        }
    }

    pub(super) fn throttle(&self, delay: Duration) {
        self.0.send_modify(|state| {
            state.limit = (state.limit / 2).max(1);
            state.successes = 0;
            state.not_before = state.not_before.max(Instant::now() + delay);
        });
    }

    pub(super) fn success(&self) {
        self.0.send_if_modified(|state| {
            state.successes += 1;
            if state.successes >= state.limit * 2 {
                state.successes = 0;
                state.limit = (state.limit + 1).min(state.ceiling);
                return true;
            }
            false
        });
    }
}

pub(super) struct Slot(Arc<Admission>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.0.send_modify(|state| state.active -= 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounds_concurrency_and_wakes_on_release_or_stop() {
        let admission = Arc::new(Admission::new(1));
        let control = RunControl::default();
        let first = admission.acquire(&control).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), admission.acquire(&control))
                .await
                .is_err()
        );
        drop(first);
        let second = admission.acquire(&control).await.unwrap();
        control.stop();
        assert!(admission.acquire(&control).await.is_none());
        drop(second);
    }

    #[test]
    fn throttling_halves_admission_then_recovers_gradually() {
        let admission = Admission::new(6);
        admission.throttle(Duration::from_secs(1));
        assert_eq!(admission.0.borrow().limit, 3);
        for _ in 0..6 {
            admission.success();
        }
        assert_eq!(admission.0.borrow().limit, 4);
        admission.configure(2);
        assert_eq!(admission.0.borrow().limit, 2);
    }
}
