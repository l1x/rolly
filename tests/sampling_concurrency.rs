#![cfg(feature = "_bench")]

//! Concurrency stress test for `should_sample`.
//! Verifies that the sampling decision is deterministic and thread-safe:
//! all threads must agree on the result for the same (trace_id, rate) pair.

use rolly::bench::should_sample;
use std::sync::{Arc, Barrier};

/// Spawn `num_threads` threads, each calling `should_sample` for every
/// (trace_id, rate) pair. Assert that all threads produce identical results.
fn assert_deterministic_across_threads(trace_ids: &[[u8; 16]], rates: &[f64], num_threads: usize) {
    let trace_ids = Arc::new(trace_ids.to_vec());
    let rates = Arc::new(rates.to_vec());
    let barrier = Arc::new(Barrier::new(num_threads));

    let handles: Vec<_> = (0..num_threads)
        .map(|_| {
            let trace_ids = Arc::clone(&trace_ids);
            let rates = Arc::clone(&rates);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait(); // synchronize start
                let mut results = Vec::new();
                for tid in trace_ids.iter() {
                    for &rate in rates.iter() {
                        results.push(should_sample(*tid, rate));
                    }
                }
                results
            })
        })
        .collect();

    let all_results: Vec<Vec<bool>> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // All threads must produce the same result vector
    let expected = &all_results[0];
    for (thread_idx, result) in all_results.iter().enumerate().skip(1) {
        assert_eq!(
            result, expected,
            "thread {} produced different results than thread 0",
            thread_idx
        );
    }
}

#[test]
fn sampling_is_deterministic_across_8_threads() {
    // Generate diverse trace_ids using BLAKE3
    let trace_ids: Vec<[u8; 16]> = (0u64..100)
        .map(|i| {
            let hash = blake3::hash(&i.to_le_bytes());
            let mut tid = [0u8; 16];
            tid.copy_from_slice(&hash.as_bytes()[..16]);
            tid
        })
        .collect();

    let rates = vec![0.0, 0.01, 0.1, 0.5, 0.99, 1.0];

    assert_deterministic_across_threads(&trace_ids, &rates, 8);
}

#[test]
fn sampling_boundary_rates_across_threads() {
    let trace_ids: Vec<[u8; 16]> = vec![
        [0x00; 16], // all zeros
        [0xFF; 16], // all ones
        [0x80; 16], // midpoint byte
        [0; 16],    // literal zero trace_id
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    ];

    let rates = vec![0.0, 1.0, f64::MIN_POSITIVE, 1.0 - f64::EPSILON];

    assert_deterministic_across_threads(&trace_ids, &rates, 8);
}

#[test]
fn sampling_single_trace_id_many_calls() {
    // Verify the same trace_id + rate always gives the same answer,
    // even under contention.
    let trace_id: [u8; 16] = {
        let hash = blake3::hash(b"stable-trace");
        let mut tid = [0u8; 16];
        tid.copy_from_slice(&hash.as_bytes()[..16]);
        tid
    };

    let expected = should_sample(trace_id, 0.5);

    let barrier = Arc::new(Barrier::new(16));
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                // Each thread calls 1000 times
                for _ in 0..1000 {
                    assert_eq!(
                        should_sample(trace_id, 0.5),
                        expected,
                        "non-deterministic sampling result"
                    );
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn sampled_out_produces_zero_channel_messages_under_concurrency() {
    use rolly::bench::{Exporter, OtlpLayer, OtlpLayerConfig};
    use tracing_subscriber::layer::SubscriberExt;

    let before = rolly::telemetry_dropped_total();
    let (exporter, mut rx) =
        Exporter::start_test_with_capacity(1024, rolly::bench::BackpressureStrategy::Drop);
    let layer = OtlpLayer::new(OtlpLayerConfig {
        exporter,
        service_name: "sample-test",
        service_version: "0.0.1",
        environment: "test",
        resource_attributes: &[],
        export_traces: true,
        export_logs: true,
        sampling_rate: 0.0,
    });
    let subscriber = tracing_subscriber::registry().with(layer);
    let dispatch = tracing::Dispatch::new(subscriber);

    let barrier = Arc::new(Barrier::new(8));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let dispatch = dispatch.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                tracing::dispatcher::with_default(&dispatch, || {
                    for _ in 0..500 {
                        let span = tracing::info_span!("sampled-out");
                        let _enter = span.enter();
                        tracing::info!("sampled-out-event");
                    }
                });
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // No messages should have been sent (sampled-out spans and their events are suppressed)
    assert!(
        rx.try_recv().is_err(),
        "expected no messages in channel when sampling_rate=0.0"
    );

    // No drops either -- sends were never attempted
    let delta = rolly::telemetry_dropped_total() - before;
    assert_eq!(
        delta, 0,
        "expected 0 drops (sends never attempted), got {}",
        delta
    );
}
