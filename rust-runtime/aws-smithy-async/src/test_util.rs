/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Test utilities for time and sleep

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use tokio::sync::oneshot;
use tokio::sync::Barrier;
use tokio::time::timeout;

use crate::rt::sleep::{AsyncSleep, Sleep};
use crate::time::{SharedTimeSource, TimeSource};

/// Manually controlled time source
#[derive(Debug, Clone)]
pub struct ManualTimeSource {
    start_time: SystemTime,
    log: Arc<Mutex<Vec<Duration>>>,
}

impl TimeSource for ManualTimeSource {
    fn now(&self) -> SystemTime {
        self.start_time + self.log.lock().unwrap().iter().sum::<Duration>()
    }
}

/// A sleep implementation where calls to [`AsyncSleep::sleep`] block until [`SleepGate::expect_sleep`] is called
///
/// Create a [`ControlledSleep`] with [`controlled_time_and_sleep`]
#[derive(Debug, Clone)]
pub struct ControlledSleep {
    barrier: Arc<Barrier>,
    log: Arc<Mutex<Vec<Duration>>>,
    duration: Arc<Mutex<Option<Duration>>>,
    advance_guard: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

/// Gate that allows [`ControlledSleep`] to advance.
///
/// See [`controlled_time_and_sleep`] for more details
pub struct SleepGate {
    gate: Arc<Barrier>,
    pending: Arc<Mutex<Option<Duration>>>,
    advance_guard: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl ControlledSleep {
    fn new(log: Arc<Mutex<Vec<Duration>>>) -> (ControlledSleep, SleepGate) {
        let gate = Arc::new(Barrier::new(2));
        let pending = Arc::new(Mutex::new(None));
        let advance_guard: Arc<Mutex<Option<oneshot::Sender<()>>>> = Default::default();
        (
            ControlledSleep {
                barrier: gate.clone(),
                log,
                duration: pending.clone(),
                advance_guard: advance_guard.clone(),
            },
            SleepGate {
                gate,
                pending,
                advance_guard,
            },
        )
    }
}

/// Guard returned from [`SleepGate::expect_sleep`]
///
/// # Examples
/// ```rust
/// # use std::sync::Arc;
/// use std::sync::atomic::{AtomicUsize, Ordering};
/// # async {
/// use std::time::{Duration, UNIX_EPOCH};
/// use aws_smithy_async::rt::sleep::AsyncSleep;
/// use aws_smithy_async::test_util::controlled_time_and_sleep;
/// let (time, sleep, mut gate) = controlled_time_and_sleep(UNIX_EPOCH);
/// let progress = Arc::new(AtomicUsize::new(0));
/// let task_progress = progress.clone();
/// let task = tokio::spawn(async move {
///     let progress = task_progress;
///     progress.store(1, Ordering::Release);
///     sleep.sleep(Duration::from_secs(1)).await;
///     progress.store(2, Ordering::Release);
///     sleep.sleep(Duration::from_secs(2)).await;
/// });
/// while progress.load(Ordering::Acquire) != 1 {}
/// let guard = gate.expect_sleep().await;
/// assert_eq!(guard.duration(), Duration::from_secs(1));
/// assert_eq!(progress.load(Ordering::Acquire), 1);
/// guard.allow_progress();
///
/// let guard = gate.expect_sleep().await;
/// assert_eq!(progress.load(Ordering::Acquire), 2);
/// assert_eq!(task.is_finished(), false);
/// guard.allow_progress();
/// task.await.expect("successful completion");
/// # };
/// ```
pub struct CapturedSleep<'a>(oneshot::Sender<()>, &'a SleepGate, Duration);
impl CapturedSleep<'_> {
    /// Allow the calling code to advance past the call to [`AsyncSleep::sleep`]
    ///
    /// In order to facilitate testing with no flakiness, the future returned by the call to `sleep`
    /// will not resolve until [`CapturedSleep`] is dropped or this method is called.
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use aws_smithy_async::rt::sleep::AsyncSleep;
    /// fn do_something(sleep: &dyn AsyncSleep) {
    ///   println!("before sleep");
    ///   sleep.sleep(Duration::from_secs(1));
    ///   println!("after sleep");
    /// }
    /// ```
    ///
    /// To be specific, when `do_something` is called, the code will advance to `sleep.sleep`.
    /// When [`SleepGate::expect_sleep`] is called, the 1 second sleep will be captured, but `after sleep`
    /// WILL NOT be printed, until `allow_progress` is called.
    pub fn allow_progress(self) {
        drop(self)
    }

    /// Duration in the call to [`AsyncSleep::sleep`]
    pub fn duration(&self) -> Duration {
        self.2
    }
}

impl AsRef<Duration> for CapturedSleep<'_> {
    fn as_ref(&self) -> &Duration {
        &self.2
    }
}

impl SleepGate {
    /// Expect the time source to sleep
    ///
    /// This returns the duration that was slept and a [`CapturedSleep`]. The drop guard is used
    /// to precisely control
    pub async fn expect_sleep(&mut self) -> CapturedSleep<'_> {
        timeout(Duration::from_secs(1), self.gate.wait())
            .await
            .expect("timeout");
        let dur = self
            .pending
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Duration::from_secs(123456));
        let guard = CapturedSleep(
            self.advance_guard.lock().unwrap().take().unwrap(),
            self,
            dur,
        );
        guard
    }
}

impl AsyncSleep for ControlledSleep {
    fn sleep(&self, duration: Duration) -> Sleep {
        let barrier = self.barrier.clone();
        let log = self.log.clone();
        let pending = self.duration.clone();
        let drop_guard = self.advance_guard.clone();
        Sleep::new(async move {
            // 1. write the duration into the shared mutex
            assert!(pending.lock().unwrap().is_none());
            *pending.lock().unwrap() = Some(duration);
            let (tx, rx) = oneshot::channel();
            *drop_guard.lock().unwrap() = Some(tx);
            // 2. first wait on the barrier—this is how we wait for an invocation of `expect_sleep`
            barrier.wait().await;
            log.lock().unwrap().push(duration);
            let _ = dbg!(rx.await);
        })
    }
}

/// Returns a trio of tools to test interactions with time
///
/// 1. [`ManualTimeSource`] which starts at a specific time and only advances when `sleep` is called.
/// It MUST be paired with [`ControlledSleep`] in order to function.
pub fn controlled_time_and_sleep(
    start_time: SystemTime,
) -> (ManualTimeSource, ControlledSleep, SleepGate) {
    let log = Arc::new(Mutex::new(vec![]));
    let (sleep, gate) = ControlledSleep::new(log.clone());
    (ManualTimeSource { start_time, log }, sleep, gate)
}

impl TimeSource for SystemTime {
    fn now(&self) -> SystemTime {
        *self
    }
}

impl From<SystemTime> for SharedTimeSource {
    fn from(value: SystemTime) -> Self {
        SharedTimeSource::new(value)
    }
}

impl From<ManualTimeSource> for SharedTimeSource {
    fn from(value: ManualTimeSource) -> Self {
        SharedTimeSource::new(value)
    }
}

#[cfg(test)]
mod test {
    use crate::rt::sleep::AsyncSleep;
    use crate::test_util::controlled_time_and_sleep;
    use crate::time::TimeSource;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::task::yield_now;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_sleep_gate() {
        use std::time::{Duration, UNIX_EPOCH};
        let start = UNIX_EPOCH;
        let (time, sleep, mut gate) = controlled_time_and_sleep(UNIX_EPOCH);
        let progress = Arc::new(AtomicUsize::new(0));
        let task_progress = progress.clone();
        let task = tokio::spawn(async move {
            assert_eq!(time.now(), start);
            let progress = task_progress;
            progress.store(1, Ordering::Release);
            sleep.sleep(Duration::from_secs(1)).await;
            assert_eq!(time.now(), start + Duration::from_secs(1));
            progress.store(2, Ordering::Release);
            sleep.sleep(Duration::from_secs(2)).await;
            assert_eq!(time.now(), start + Duration::from_secs(3));
        });
        while progress.load(Ordering::Acquire) != 1 {
            yield_now().await
        }
        let guard = gate.expect_sleep().await;
        assert_eq!(guard.duration(), Duration::from_secs(1));
        assert_eq!(progress.load(Ordering::Acquire), 1);
        guard.allow_progress();

        let guard = gate.expect_sleep().await;
        assert_eq!(progress.load(Ordering::Acquire), 2);
        assert_eq!(task.is_finished(), false);
        guard.allow_progress();
        timeout(Duration::from_secs(1), task)
            .await
            .expect("no timeout")
            .expect("successful completion");
    }
}
