#![cfg(feature = "_bench")]
//! Measures Counter::add throughput under multi-threaded contention.
//! Run with: cargo test --features _bench --release --test mutex_contention -- --nocapture

use rolly::bench::MetricsRegistry;
use std::sync::Arc;
use std::time::{Duration, Instant};

const OPS_PER_THREAD: u64 = 2_000_000;
const WARMUP_OPS: u64 = 100_000;

fn measure_throughput(num_threads: usize, use_attrs: bool) -> (Duration, f64) {
    let registry = Arc::new(MetricsRegistry::new());
    let counter = registry.counter("bench_counter", "contention test");

    // Warmup
    for _ in 0..WARMUP_OPS {
        if use_attrs {
            counter.add(1, &[("method", "GET"), ("status", "200")]);
        } else {
            counter.add(1, &[]);
        }
    }

    let barrier = Arc::new(std::sync::Barrier::new(num_threads));

    let handles: Vec<_> = (0..num_threads)
        .map(|_| {
            let counter = counter.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let start = Instant::now();
                for _ in 0..OPS_PER_THREAD {
                    if use_attrs {
                        counter.add(1, &[("method", "GET"), ("status", "200")]);
                    } else {
                        counter.add(1, &[]);
                    }
                }
                start.elapsed()
            })
        })
        .collect();

    let durations: Vec<Duration> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let max_duration = durations.iter().max().unwrap();
    let total_ops = OPS_PER_THREAD * num_threads as u64;
    let throughput = total_ops as f64 / max_duration.as_secs_f64();

    (*max_duration, throughput)
}

#[test]
fn counter_contention_scaling() {
    let thread_counts = [1, 2, 4, 8];

    println!("\n--- Counter::add contention (no attrs) ---");
    println!(
        "{:>8} {:>12} {:>14} {:>10}",
        "threads", "duration", "throughput", "ns/op"
    );
    println!("{}", "-".repeat(48));
    for &n in &thread_counts {
        let (dur, throughput) = measure_throughput(n, false);
        let ns_per_op = dur.as_nanos() as f64 / OPS_PER_THREAD as f64;
        println!(
            "{:>8} {:>10.1}ms {:>11.1}M/s {:>8.1}ns",
            n,
            dur.as_secs_f64() * 1000.0,
            throughput / 1_000_000.0,
            ns_per_op
        );
    }

    println!("\n--- Counter::add contention (2 attrs) ---");
    println!(
        "{:>8} {:>12} {:>14} {:>10}",
        "threads", "duration", "throughput", "ns/op"
    );
    println!("{}", "-".repeat(48));
    for &n in &thread_counts {
        let (dur, throughput) = measure_throughput(n, true);
        let ns_per_op = dur.as_nanos() as f64 / OPS_PER_THREAD as f64;
        println!(
            "{:>8} {:>10.1}ms {:>11.1}M/s {:>8.1}ns",
            n,
            dur.as_secs_f64() * 1000.0,
            throughput / 1_000_000.0,
            ns_per_op
        );
    }
}
