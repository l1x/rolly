#![cfg(feature = "_bench")]

//! Backpressure tests: prove the hot path never blocks when the export channel is full.

use std::sync::{Arc, Barrier};
use std::time::Instant;

use rolly::bench::{BackpressureStrategy, Exporter, OtlpLayer};
use tracing_subscriber::layer::SubscriberExt;

/// Create a Dispatch backed by an OtlpLayer with a test exporter of the given capacity
/// and backpressure strategy.
/// Returns the dispatch and the receiver (hold it to keep the channel open).
fn make_dispatch(
    capacity: usize,
    export_traces: bool,
    export_logs: bool,
    sampling_rate: f64,
    strategy: BackpressureStrategy,
) -> (
    tracing::Dispatch,
    tokio::sync::mpsc::Receiver<rolly::bench::ExportMessage>,
) {
    let (exporter, rx) = Exporter::start_test_with_capacity(capacity, strategy);
    let layer = OtlpLayer::new(
        exporter,
        "bp-test",
        "0.0.1",
        "test",
        export_traces,
        export_logs,
        sampling_rate,
    );
    let subscriber = tracing_subscriber::registry().with(layer);
    (tracing::Dispatch::new(subscriber), rx)
}

#[test]
fn hot_path_does_not_block_when_channel_full() {
    let before = rolly::telemetry_dropped_total();
    let (dispatch, _rx) = make_dispatch(4, true, false, 1.0, BackpressureStrategy::Drop);

    let start = Instant::now();
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..1000 {
            let span = tracing::info_span!("bp-span");
            let _enter = span.enter();
        }
    });
    let elapsed = start.elapsed();

    let delta = rolly::telemetry_dropped_total() - before;
    assert!(
        elapsed.as_millis() < 500,
        "hot path blocked: took {}ms (limit 500ms)",
        elapsed.as_millis()
    );
    assert!(
        delta >= 996,
        "expected at least 996 drops (1000 spans - 4 capacity), got {}",
        delta
    );
}

#[test]
fn hot_path_nonblocking_under_concurrent_pressure() {
    let before = rolly::telemetry_dropped_total();
    let (dispatch, _rx) = make_dispatch(4, true, false, 1.0, BackpressureStrategy::Drop);
    let barrier = Arc::new(Barrier::new(8));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let dispatch = dispatch.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let start = Instant::now();
                tracing::dispatcher::with_default(&dispatch, || {
                    for _ in 0..500 {
                        let span = tracing::info_span!("pressure-span");
                        let _enter = span.enter();
                    }
                });
                start.elapsed()
            })
        })
        .collect();

    let max_elapsed = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .max()
        .unwrap();

    let delta = rolly::telemetry_dropped_total() - before;
    assert!(
        max_elapsed.as_secs() < 2,
        "slowest thread took {:?} (limit 2s)",
        max_elapsed
    );
    // 8 threads x 500 spans = 4000 total, minus up to 4 that fit in channel
    assert!(delta >= 3996, "expected at least 3996 drops, got {}", delta);
}

#[test]
fn dropped_total_increments_accurately() {
    let (dispatch, _rx) = make_dispatch(1, true, false, 1.0, BackpressureStrategy::Drop);

    // Fill the single-capacity channel with one span
    tracing::dispatcher::with_default(&dispatch, || {
        let span = tracing::info_span!("fill-span");
        let _enter = span.enter();
    });

    // Channel is now full -- next 100 sends should all be dropped
    let before = rolly::telemetry_dropped_total();
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..100 {
            let span = tracing::info_span!("drop-span");
            let _enter = span.enter();
        }
    });

    let delta = rolly::telemetry_dropped_total() - before;
    // Use >= because the global AtomicU64 may be incremented by parallel tests
    assert!(delta >= 100, "expected at least 100 drops, got {}", delta);
}

#[test]
fn per_span_p99_latency_stays_bounded_under_backpressure() {
    let (dispatch, _rx) = make_dispatch(4, true, false, 1.0, BackpressureStrategy::Drop);

    // Pre-fill the channel so all subsequent sends hit try_send failure path.
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..4 {
            let span = tracing::info_span!("fill-span");
            let _enter = span.enter();
        }
    });

    // Measure individual span creation latencies under sustained backpressure.
    let mut latencies = Vec::with_capacity(5000);
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..5000 {
            let t0 = Instant::now();
            let span = tracing::info_span!("latency-span", attr = "value");
            let _enter = span.enter();
            drop(_enter);
            drop(span);
            latencies.push(t0.elapsed());
        }
    });

    latencies.sort();
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[latencies.len() * 99 / 100];
    let max = latencies[latencies.len() - 1];

    // The hot path is try_send (non-blocking). p99 should be well under 1ms.
    // Use 1ms as a generous upper bound (typically ~1-10µs).
    assert!(
        p99.as_micros() < 1000,
        "p99 span latency under backpressure is {}µs (limit 1000µs). p50={}µs max={}µs",
        p99.as_micros(),
        p50.as_micros(),
        max.as_micros(),
    );

    eprintln!(
        "  latency under backpressure: p50={:?} p99={:?} max={:?}",
        p50, p99, max,
    );
}

#[test]
fn no_thread_starves_under_sustained_backpressure() {
    let (dispatch, _rx) = make_dispatch(4, true, true, 1.0, BackpressureStrategy::Drop);
    let barrier = Arc::new(Barrier::new(8));
    let spans_per_thread = 2000;

    let handles: Vec<_> = (0..8)
        .map(|thread_id| {
            let dispatch = dispatch.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let start = Instant::now();
                tracing::dispatcher::with_default(&dispatch, || {
                    for i in 0..spans_per_thread {
                        let span = tracing::info_span!("starvation-span", tid = thread_id, seq = i);
                        let _enter = span.enter();
                        tracing::info!("starvation-event");
                    }
                });
                start.elapsed()
            })
        })
        .collect();

    let thread_times: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let min = *thread_times.iter().min().unwrap();
    let max = *thread_times.iter().max().unwrap();

    // No thread should take more than 5x the fastest thread (fairness bound).
    // In practice they should be within 2x. 5x is a generous non-flaky bound.
    assert!(
        max.as_micros() < min.as_micros() * 5 + 1000, // +1ms for measurement noise
        "thread starvation detected: fastest={:?} slowest={:?} (ratio {:.1}x, limit 5x)",
        min,
        max,
        max.as_secs_f64() / min.as_secs_f64(),
    );

    // All threads must finish within 3 seconds total.
    assert!(
        max.as_secs() < 3,
        "slowest thread took {:?} (limit 3s)",
        max,
    );

    eprintln!(
        "  8 threads x {} spans+events: fastest={:?} slowest={:?} ratio={:.2}x",
        spans_per_thread,
        min,
        max,
        max.as_secs_f64() / min.as_secs_f64(),
    );
}

#[test]
fn events_also_nonblocking_under_backpressure() {
    let before = rolly::telemetry_dropped_total();
    let (dispatch, _rx) = make_dispatch(2, false, true, 1.0, BackpressureStrategy::Drop);

    let start = Instant::now();
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..1000 {
            tracing::info!("backpressure-event");
        }
    });
    let elapsed = start.elapsed();

    let delta = rolly::telemetry_dropped_total() - before;
    assert!(
        elapsed.as_millis() < 500,
        "event hot path blocked: took {}ms (limit 500ms)",
        elapsed.as_millis()
    );
    assert!(
        delta >= 998,
        "expected at least 998 drops (1000 events - 2 capacity), got {}",
        delta
    );
}
