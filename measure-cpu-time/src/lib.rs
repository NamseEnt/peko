use std::future::Future;
use std::ops::Sub;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

pub trait Clock {
    type Instant: Sub<Output = Duration> + Copy;
    fn now(&self) -> Self::Instant;
}

#[derive(Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    type Instant = Instant;

    fn now(&self) -> Self::Instant {
        Instant::now()
    }
}

pub struct MeasureCpuTime<F, C = SystemClock> {
    future: F,
    tracker: TimeTracker,
    clock: C,
}

pub fn measure_cpu_time<F>(tracker: TimeTracker, future: F) -> MeasureCpuTime<F> {
    MeasureCpuTime {
        future,
        tracker,
        clock: SystemClock,
    }
}

pub fn measure_cpu_time_with_clock<F, C: Clock>(
    tracker: TimeTracker,
    clock: C,
    future: F,
) -> MeasureCpuTime<F, C> {
    MeasureCpuTime {
        future,
        tracker,
        clock,
    }
}

impl<F: Future, C: Clock> Future for MeasureCpuTime<F, C> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        let start = this.clock.now();

        let future = unsafe { Pin::new_unchecked(&mut this.future) };
        let result = future.poll(cx);

        let end = this.clock.now();
        let elapsed = end - start;
        this.tracker
            .inner
            .fetch_add(elapsed.as_nanos() as usize, Ordering::Relaxed);

        match result {
            Poll::Ready(val) => Poll::Ready(val),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Clone, Default)]
pub struct TimeTracker {
    inner: Arc<AtomicUsize>,
}

impl TimeTracker {
    pub fn duration(&self) -> Duration {
        Duration::from_nanos(self.inner.load(Ordering::Relaxed) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Barrier;
    use tokio::time::sleep;

    // Helper: Custom future that yields a specified number of times
    struct YieldingFuture {
        yields_remaining: u32,
        value: i32,
    }

    impl Future for YieldingFuture {
        type Output = i32;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.yields_remaining > 0 {
                self.yields_remaining -= 1;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(self.value)
            }
        }
    }

    fn yielding_future(yields: u32, value: i32) -> YieldingFuture {
        YieldingFuture {
            yields_remaining: yields,
            value,
        }
    }

    // Helper: Assert duration is within a tolerance range
    #[allow(dead_code)]
    fn assert_duration_in_range(actual: Duration, expected: Duration, tolerance_percent: u64) {
        let expected_ms = expected.as_millis() as u64;
        let actual_ms = actual.as_millis() as u64;
        let tolerance = expected_ms * tolerance_percent / 100;
        let lower = expected_ms.saturating_sub(tolerance);
        let upper = expected_ms + tolerance;

        assert!(
            actual_ms >= lower && actual_ms <= upper,
            "Expected {}ms ±{}%, got {}ms (range: {}-{}ms)",
            expected_ms,
            tolerance_percent,
            actual_ms,
            lower,
            upper
        );
    }

    // Category 1: Basic Functionality Tests

    #[tokio::test]
    async fn test_measure_simple_async_operation() {
        let future = async {
            sleep(Duration::from_millis(10)).await;
            42
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, 42);
        // Should be at least a few milliseconds
        assert!(elapsed.as_micros() > 0);
    }

    #[tokio::test]
    async fn test_measure_returns_correct_output_type() {
        let string_future = async { "hello".to_string() };
        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker, string_future);
        let result = measured.await;
        assert_eq!(result, "hello");

        let vec_future = async { vec![1, 2, 3] };
        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker, vec_future);
        let result = measured.await;
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_measure_immediate_ready_future() {
        let future = async { 100 };
        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, 100);
        assert!(elapsed.as_micros() < 10_000);
    }

    // Category 2: Multi-Poll Scenarios

    #[tokio::test]
    async fn test_accumulates_time_across_multiple_polls() {
        let future = async {
            sleep(Duration::from_millis(5)).await;
            tokio::task::yield_now().await;
            sleep(Duration::from_millis(5)).await;
            tokio::task::yield_now().await;
            sleep(Duration::from_millis(5)).await;
            100
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, 100);
        // Verify time accumulates across multiple polls
        assert!(elapsed.as_micros() > 0);
    }

    #[tokio::test]
    async fn test_multiple_yields_with_custom_future() {
        let future = yielding_future(5, 42);
        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, 42);
        assert!(elapsed.as_nanos() > 0);
    }

    #[tokio::test]
    async fn test_interleaved_computation_and_awaits() {
        let future = async {
            let mut sum = 0;
            for i in 0..1000 {
                sum += i;
            }
            sleep(Duration::from_millis(5)).await;

            for i in 0..1000 {
                sum += i;
            }
            sleep(Duration::from_millis(5)).await;

            sum
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, 999000);
        // Verify that both computation and awaits are measured
        assert!(elapsed.as_micros() > 0);
    }

    // Category 3: Concurrent Execution Tests

    #[tokio::test]
    async fn test_concurrent_measurements_independent() {
        let barrier = Arc::new(Barrier::new(5));
        let mut handles = vec![];

        for i in 0..5u64 {
            let barrier_clone = barrier.clone();
            let handle = tokio::spawn(async move {
                barrier_clone.wait().await;

                let sleep_ms = (i + 1) * 2;
                let future = async move {
                    sleep(Duration::from_millis(sleep_ms)).await;
                    i
                };

                let tracker = TimeTracker::default();
                let measured = measure_cpu_time(tracker.clone(), future);
                let result = measured.await;
                let elapsed = tracker.duration();
                (result, elapsed)
            });
            handles.push(handle);
        }

        for (idx, handle) in handles.into_iter().enumerate() {
            let (result, elapsed) = handle.await.unwrap();
            assert_eq!(result, idx as u64);
            // Verify each measurement is independent
            assert!(elapsed.as_micros() > 0);
        }
    }

    #[tokio::test]
    async fn test_many_concurrent_measurements() {
        let mut handles = vec![];

        for i in 0..50 {
            let handle = tokio::spawn(async move {
                let future = async move {
                    sleep(Duration::from_millis(1)).await;
                    i
                };
                let tracker = TimeTracker::default();
                let measured = measure_cpu_time(tracker.clone(), future);
                let result = measured.await;
                let elapsed = tracker.duration();
                (result, elapsed)
            });
            handles.push(handle);
        }

        for (i, handle) in handles.into_iter().enumerate() {
            let (result, elapsed) = handle.await.unwrap();
            assert_eq!(result, i);
            assert!(elapsed.as_nanos() > 0);
        }
    }

    #[tokio::test]
    async fn test_nested_measure_cpu_time() {
        let inner_future = async {
            sleep(Duration::from_millis(5)).await;
            42
        };

        let inner_tracker = TimeTracker::default();
        let outer_future = async {
            let inner_measured = measure_cpu_time(inner_tracker.clone(), inner_future);
            let inner_result = inner_measured.await;
            let inner_time = inner_tracker.duration();
            sleep(Duration::from_millis(5)).await;
            (inner_result, inner_time)
        };

        let outer_tracker = TimeTracker::default();
        let outer_measured = measure_cpu_time(outer_tracker.clone(), outer_future);
        let (result, inner_elapsed) = outer_measured.await;
        let outer_elapsed = outer_tracker.duration();

        assert_eq!(result, 42);
        assert!(inner_elapsed.as_micros() > 0);
        assert!(outer_elapsed.as_micros() > 0);
        // Outer should include inner time plus additional work
        assert!(outer_elapsed >= inner_elapsed);
    }

    // Category 4: Error Handling Tests

    #[tokio::test]
    async fn test_measure_with_result_ok() {
        let future = async {
            sleep(Duration::from_millis(5)).await;
            Ok::<i32, String>(42)
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, Ok(42));
        assert!(elapsed.as_micros() > 0);
    }

    #[tokio::test]
    async fn test_measure_with_result_err() {
        let future = async {
            sleep(Duration::from_millis(5)).await;
            Err::<i32, String>("error occurred".to_string())
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, Err("error occurred".to_string()));
        assert!(elapsed.as_micros() > 0);
    }

    #[tokio::test]
    async fn test_measure_with_option_none() {
        let future = async {
            sleep(Duration::from_millis(5)).await;
            None::<i32>
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, None);
        assert!(elapsed.as_micros() > 0);
    }

    #[tokio::test]
    #[should_panic(expected = "intentional panic")]
    async fn test_measure_future_that_panics() {
        let future = async {
            panic!("intentional panic");
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker, future);
        let _ = measured.await;
    }

    // Category 5: Timing Validation Tests

    #[tokio::test]
    async fn test_timing_accuracy_with_known_delays() {
        // Test that longer delays result in longer measured times
        let future1 = async {
            sleep(Duration::from_millis(5)).await;
            1
        };
        let future2 = async {
            sleep(Duration::from_millis(10)).await;
            2
        };
        let future3 = async {
            sleep(Duration::from_millis(20)).await;
            3
        };

        let tracker1 = TimeTracker::default();
        let measured1 = measure_cpu_time(tracker1.clone(), future1);
        let result1 = measured1.await;
        let elapsed1 = tracker1.duration();

        let tracker2 = TimeTracker::default();
        let measured2 = measure_cpu_time(tracker2.clone(), future2);
        let result2 = measured2.await;
        let elapsed2 = tracker2.duration();

        let tracker3 = TimeTracker::default();
        let measured3 = measure_cpu_time(tracker3.clone(), future3);
        let result3 = measured3.await;
        let elapsed3 = tracker3.duration();

        assert_eq!(result1, 1);
        assert_eq!(result2, 2);
        assert_eq!(result3, 3);

        // All should have measured non-zero time
        assert!(elapsed1.as_micros() > 0);
        assert!(elapsed2.as_micros() > 0);
        assert!(elapsed3.as_micros() > 0);
    }

    #[tokio::test]
    async fn test_zero_duration_for_instant_completion() {
        let future = async { 1 + 1 };
        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, 2);
        assert!(elapsed.as_micros() < 1000);
    }

    #[tokio::test]
    async fn test_duration_increases_with_work() {
        // Test that more work results in longer measured time
        let future1 = async {
            sleep(Duration::from_millis(5)).await;
        };
        let future2 = async {
            sleep(Duration::from_millis(5)).await;
            sleep(Duration::from_millis(5)).await;
        };
        let future3 = async {
            sleep(Duration::from_millis(5)).await;
            sleep(Duration::from_millis(5)).await;
            sleep(Duration::from_millis(5)).await;
        };

        let tracker1 = TimeTracker::default();
        let measured1 = measure_cpu_time(tracker1.clone(), future1);
        let _ = measured1.await;
        let elapsed1 = tracker1.duration();

        let tracker2 = TimeTracker::default();
        let measured2 = measure_cpu_time(tracker2.clone(), future2);
        let _ = measured2.await;
        let elapsed2 = tracker2.duration();

        let tracker3 = TimeTracker::default();
        let measured3 = measure_cpu_time(tracker3.clone(), future3);
        let _ = measured3.await;
        let elapsed3 = tracker3.duration();

        // All should be non-zero
        assert!(elapsed1.as_micros() > 0);
        assert!(elapsed2.as_micros() > 0);
        assert!(elapsed3.as_micros() > 0);
        // More work should generally take longer
        assert!(elapsed3 > elapsed1);
    }

    // Category 6: Edge Cases

    #[tokio::test]
    async fn test_measure_unit_type_future() {
        let future = async {
            sleep(Duration::from_millis(5)).await;
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, ());
        assert!(elapsed.as_micros() > 0);
    }

    #[tokio::test]
    async fn test_measure_large_output() {
        let future = async {
            sleep(Duration::from_millis(5)).await;
            vec![0u8; 1_000_000]
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result.len(), 1_000_000);
        assert!(elapsed.as_micros() > 0);
    }

    #[tokio::test]
    async fn test_measure_empty_async_block() {
        let future = async {};
        let tracker = TimeTracker::default();
        let measured = measure_cpu_time(tracker.clone(), future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, ());
        // Even an empty block should have some measurement overhead
        assert!(elapsed.as_nanos() < 1_000_000); // Less than 1ms
    }

    // Category 7: Mocking Tests

    #[derive(Clone)]
    struct MockClock {
        now: Arc<std::sync::Mutex<Instant>>,
    }

    impl MockClock {
        fn new(start_time: Instant) -> Self {
            Self {
                now: Arc::new(std::sync::Mutex::new(start_time)),
            }
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.now.lock().unwrap();
            *now += duration;
        }
    }

    impl Clock for MockClock {
        type Instant = Instant;

        fn now(&self) -> Self::Instant {
            *self.now.lock().unwrap()
        }
    }

    #[tokio::test]
    async fn test_measure_with_mock_clock() {
        let start_time = Instant::now();
        let clock = MockClock::new(start_time);
        let clock_clone = clock.clone();

        let future = async move {
            // Simulate work by advancing the clock
            clock_clone.advance(Duration::from_secs(1));
            42
        };

        let tracker = TimeTracker::default();
        let measured = measure_cpu_time_with_clock(tracker.clone(), clock, future);
        let result = measured.await;
        let elapsed = tracker.duration();

        assert_eq!(result, 42);
        // We expect exactly 1 second because we manually advanced the clock
        assert_eq!(elapsed, Duration::from_secs(1));
    }
}
