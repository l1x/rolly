//! Generate flamechart SVGs, differential flamegraph, allocation counts, and
//! latency measurements for ro11y metrics hot-path analysis.
//!
//! Produces in `docs/flamecharts/`:
//! - `flamechart_before.svg`  — old ro11y Counter::add (pre-optimization)
//! - `flamechart_after.svg`   — optimized ro11y Counter::add
//! - `flamechart_otel.svg`    — OpenTelemetry SDK 0.31 Counter::add
//! - `flamechart_diff.svg`    — differential: before → after
//!
//! Also prints allocation counts and latency per operation.
//!
//! Run: `cargo bench --features _bench --bench generate_flamecharts`
//!
//! Requires macOS `sample` command (ships with Xcode CLI tools).

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::io::{BufReader, Cursor};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ── Counting allocator ─────────────────────────────────────────────────

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn reset_alloc() {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    DEALLOC_COUNT.store(0, Ordering::Relaxed);
}

fn read_alloc() -> (u64, u64, u64) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
        DEALLOC_COUNT.load(Ordering::Relaxed),
    )
}

// ── Constants ──────────────────────────────────────────────────────────

const PROFILE_SECS: u64 = 12;
const SAMPLE_DELAY_SECS: u64 = 3;
const SAMPLE_SECS: &str = "7";
const MEASURE_ITERS: u64 = 1_000_000;
const WARMUP_ITERS: u64 = 10_000;

// ── Main ───────────────────────────────────────────────────────────────

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--before") => profile_before(),
        Some("--after") => profile_after(),
        Some("--otel") => profile_otel(),
        Some("--measure") => measure_all(),
        _ => generate_all(),
    }
}

// ── Before (old algorithm) ─────────────────────────────────────────────

fn profile_before() {
    use std::collections::HashMap;
    use std::sync::Mutex;

    let data: Mutex<HashMap<u64, i64>> = Mutex::new(HashMap::new());
    let warmup: &[(&str, &str)] = &[("method", "GET"), ("status", "200"), ("region", "us-east-1")];
    {
        let mut sorted = attrs_to_vec(warmup);
        sort_attrs(&mut sorted);
        let key = attrs_hash_siphash(&sorted);
        data.lock().unwrap().insert(key, 0);
    }

    let deadline = Instant::now() + Duration::from_secs(PROFILE_SECS);
    while Instant::now() < deadline {
        for _ in 0..1000 {
            let attrs =
                black_box(&[("method", "GET"), ("status", "200"), ("region", "us-east-1")][..]);
            let value = black_box(1u64);
            capture_exemplar_span_current();
            let mut sorted = attrs_to_vec(attrs);
            sort_attrs(&mut sorted);
            let key = attrs_hash_siphash(&sorted);
            let mut map = data.lock().unwrap();
            *map.entry(key).or_insert(0) += value as i64;
        }
    }
}

#[inline(never)]
fn capture_exemplar_span_current() {
    let _span = black_box(tracing::Span::current());
}

#[inline(never)]
fn attrs_to_vec<'a>(attrs: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    attrs.to_vec()
}

#[inline(never)]
fn sort_attrs(attrs: &mut [(&str, &str)]) {
    attrs.sort();
}

#[inline(never)]
fn attrs_hash_siphash(attrs: &[(&str, &str)]) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for (k, v) in attrs {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }
    hasher.finish()
}

// ── After (optimized) ──────────────────────────────────────────────────

fn profile_after() {
    use ro11y::bench::*;
    let registry = MetricsRegistry::new();
    let counter = registry.counter("test", "test");
    counter.add(
        1,
        &[("method", "GET"), ("status", "200"), ("region", "us-east-1")],
    );

    let deadline = Instant::now() + Duration::from_secs(PROFILE_SECS);
    while Instant::now() < deadline {
        for _ in 0..1000 {
            counter.add(
                black_box(1),
                black_box(&[("method", "GET"), ("status", "200"), ("region", "us-east-1")]),
            );
        }
    }
}

// ── OTel SDK ───────────────────────────────────────────────────────────

fn profile_otel() {
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::metrics::{ManualReader, SdkMeterProvider};

    let provider = SdkMeterProvider::builder()
        .with_reader(ManualReader::builder().build())
        .build();
    let meter = provider.meter("bench");
    let ctr = meter.u64_counter("test").build();
    ctr.add(
        1,
        &[
            KeyValue::new("method", "GET"),
            KeyValue::new("status", "200"),
            KeyValue::new("region", "us-east-1"),
        ],
    );

    let deadline = Instant::now() + Duration::from_secs(PROFILE_SECS);
    while Instant::now() < deadline {
        for _ in 0..1000 {
            ctr.add(
                black_box(1),
                black_box(&[
                    KeyValue::new("method", "GET"),
                    KeyValue::new("status", "200"),
                    KeyValue::new("region", "us-east-1"),
                ]),
            );
        }
    }
}

// ── Allocation + latency measurement ───────────────────────────────────

fn measure_all() {
    let attrs_ro11y: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
    ];

    eprintln!(
        "\n{:=<74}",
        "= Allocation & Latency Measurement (Counter::add, 3 attrs) "
    );
    eprintln!(
        "  {} iterations per variant, {} warmup\n",
        MEASURE_ITERS, WARMUP_ITERS
    );

    // ── Before ──
    let before = {
        use std::collections::HashMap;
        use std::hash::{DefaultHasher, Hash, Hasher};
        use std::sync::Mutex;

        let data: Mutex<HashMap<u64, i64>> = Mutex::new(HashMap::new());
        // warmup — insert entry so hot path hits existing key
        for _ in 0..WARMUP_ITERS {
            let mut sorted = attrs_ro11y.to_vec();
            sorted.sort();
            let mut h = DefaultHasher::new();
            for (k, v) in &sorted {
                k.hash(&mut h);
                v.hash(&mut h);
            }
            let key = h.finish();
            let mut map = data.lock().unwrap();
            *map.entry(key).or_insert(0) += 1i64;
        }

        reset_alloc();
        let start = Instant::now();
        for _ in 0..MEASURE_ITERS {
            let attrs = black_box(attrs_ro11y);
            let value = black_box(1u64);
            let _span = tracing::Span::current();
            let mut sorted = attrs.to_vec();
            sorted.sort();
            let mut h = DefaultHasher::new();
            for (k, v) in &sorted {
                k.hash(&mut h);
                v.hash(&mut h);
            }
            let key = h.finish();
            let mut map = data.lock().unwrap();
            *map.entry(key).or_insert(0) += value as i64;
        }
        let elapsed = start.elapsed();
        let (ac, ab, dc) = read_alloc();
        MeasureResult {
            label: "Before (old ro11y)",
            elapsed,
            allocs: ac,
            alloc_bytes: ab,
            deallocs: dc,
        }
    };

    // ── After ──
    let after = {
        use ro11y::bench::*;
        let registry = MetricsRegistry::new();
        let counter = registry.counter("test", "test");
        for _ in 0..WARMUP_ITERS {
            counter.add(1, attrs_ro11y);
        }

        reset_alloc();
        let start = Instant::now();
        for _ in 0..MEASURE_ITERS {
            counter.add(black_box(1), black_box(attrs_ro11y));
        }
        let elapsed = start.elapsed();
        let (ac, ab, dc) = read_alloc();
        MeasureResult {
            label: "After (optimized ro11y)",
            elapsed,
            allocs: ac,
            alloc_bytes: ab,
            deallocs: dc,
        }
    };

    // ── OTel ──
    let otel = {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry::KeyValue;
        use opentelemetry_sdk::metrics::{ManualReader, SdkMeterProvider};

        let provider = SdkMeterProvider::builder()
            .with_reader(ManualReader::builder().build())
            .build();
        let meter = provider.meter("bench");
        let ctr = meter.u64_counter("test").build();
        let otel_attrs = [
            KeyValue::new("method", "GET"),
            KeyValue::new("status", "200"),
            KeyValue::new("region", "us-east-1"),
        ];
        for _ in 0..WARMUP_ITERS {
            ctr.add(1, &otel_attrs);
        }

        reset_alloc();
        let start = Instant::now();
        for _ in 0..MEASURE_ITERS {
            ctr.add(
                black_box(1),
                black_box(&[
                    KeyValue::new("method", "GET"),
                    KeyValue::new("status", "200"),
                    KeyValue::new("region", "us-east-1"),
                ]),
            );
        }
        let elapsed = start.elapsed();
        let (ac, ab, dc) = read_alloc();
        MeasureResult {
            label: "OTel SDK 0.31",
            elapsed,
            allocs: ac,
            alloc_bytes: ab,
            deallocs: dc,
        }
    };

    // ── Print results ──
    eprintln!(
        "  {:<24} {:>10} {:>12} {:>14} {:>12}",
        "Variant", "Latency", "Allocs/op", "Bytes/op", "Frees/op"
    );
    eprintln!("  {:-<72}", "");
    for r in [&before, &after, &otel] {
        r.print();
    }
    eprintln!();
}

struct MeasureResult {
    label: &'static str,
    elapsed: Duration,
    allocs: u64,
    alloc_bytes: u64,
    deallocs: u64,
}

impl MeasureResult {
    fn print(&self) {
        let ns_per_op = self.elapsed.as_nanos() as f64 / MEASURE_ITERS as f64;
        let allocs_per_op = self.allocs as f64 / MEASURE_ITERS as f64;
        let bytes_per_op = self.alloc_bytes as f64 / MEASURE_ITERS as f64;
        let frees_per_op = self.deallocs as f64 / MEASURE_ITERS as f64;
        eprintln!(
            "  {:<24} {:>7.1} ns {:>10.2}/op {:>10.0} B/op {:>10.2}/op",
            self.label, ns_per_op, allocs_per_op, bytes_per_op, frees_per_op
        );
    }
}

// ── Flamechart generation ──────────────────────────────────────────────

fn generate_all() {
    use inferno::collapse::{sample, Collapse};
    use inferno::flamegraph;

    std::fs::create_dir_all("docs/flamecharts").expect("create docs/flamecharts");
    let exe = std::env::current_exe().expect("current_exe");

    let variants: &[(&str, &str, &str)] = &[
        (
            "--before",
            "flamechart_before.svg",
            "ro11y Counter::add (3 attrs) \u{2014} Before Optimization",
        ),
        (
            "--after",
            "flamechart_after.svg",
            "ro11y Counter::add (3 attrs) \u{2014} After Optimization",
        ),
        (
            "--otel",
            "flamechart_otel.svg",
            "OpenTelemetry SDK 0.31 Counter::add (3 attrs)",
        ),
    ];

    // Collect cleaned collapsed stacks for differential flamegraph
    let mut collapsed_by_tag: Vec<(String, String)> = Vec::new();

    for (arg, filename, title) in variants {
        let svg_path = format!("docs/flamecharts/{}", filename);
        let tag = arg.trim_start_matches("--");
        let sample_path = format!("/tmp/ro11y_flamechart_{}.txt", tag);

        eprintln!("\n=== {} ===", title);

        // 1. Spawn self in profile mode
        let mut child = Command::new(&exe)
            .arg(arg)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {}: {}", arg, e));
        let pid = child.id();
        eprintln!("  PID {} running for {}s", pid, PROFILE_SECS);

        // 2. Wait for warmup
        std::thread::sleep(Duration::from_secs(SAMPLE_DELAY_SECS));

        // 3. Capture stack samples via macOS `sample` command
        eprintln!("  Sampling for {}s ...", SAMPLE_SECS);
        let sample_out = Command::new("sample")
            .args([&pid.to_string(), SAMPLE_SECS, "-file", &sample_path])
            .output()
            .expect("run `sample` \u{2014} is Xcode CLI tools installed?");

        if !sample_out.status.success() {
            eprintln!(
                "  WARNING: sample exit {:?}: {}",
                sample_out.status,
                String::from_utf8_lossy(&sample_out.stdout)
            );
        }

        let _ = child.wait();

        // 4. Read raw sample data
        let raw =
            std::fs::read(&sample_path).unwrap_or_else(|e| panic!("read {}: {}", sample_path, e));
        eprintln!("  Raw sample: {} bytes", raw.len());

        // 5. Collapse stacks
        let mut folder = sample::Folder::from(sample::Options::default());
        let mut collapsed_bytes = Vec::new();
        folder
            .collapse(BufReader::new(Cursor::new(&raw)), &mut collapsed_bytes)
            .expect("collapse sample data");

        // 6. Clean up frame names for readability
        let collapsed = String::from_utf8(collapsed_bytes).expect("utf8");
        let cleaned = clean_frames(&collapsed);
        collapsed_by_tag.push((tag.to_string(), cleaned.clone()));

        // 7. Render flamegraph SVG
        let mut opts = flamegraph::Options::default();
        opts.title = title.to_string();
        opts.count_name = "samples".into();
        opts.min_width = 0.1;

        let mut svg = Vec::new();
        flamegraph::from_reader(
            &mut opts,
            BufReader::new(Cursor::new(cleaned.as_bytes())),
            &mut svg,
        )
        .expect("render flamegraph");

        std::fs::write(&svg_path, &svg).expect("write svg");

        // 8. Verify and summarize
        let svg_str = String::from_utf8_lossy(&svg);
        verify_svg(&svg_str, &svg_path);
    }

    // ── Differential flamegraph: before → after ────────────────────────
    let before_stacks = collapsed_by_tag
        .iter()
        .find(|(t, _)| t == "before")
        .map(|(_, s)| s.as_str())
        .unwrap_or("");
    let after_stacks = collapsed_by_tag
        .iter()
        .find(|(t, _)| t == "after")
        .map(|(_, s)| s.as_str())
        .unwrap_or("");

    if !before_stacks.is_empty() && !after_stacks.is_empty() {
        eprintln!("\n=== Differential flamegraph: before \u{2192} after ===");

        let mut diff_output = Vec::new();
        inferno::differential::from_readers(
            inferno::differential::Options::default(),
            BufReader::new(Cursor::new(before_stacks.as_bytes())),
            BufReader::new(Cursor::new(after_stacks.as_bytes())),
            &mut diff_output,
        )
        .expect("compute differential");

        let mut opts = flamegraph::Options::default();
        opts.title = "Differential: before \u{2192} after (red=growth, blue=shrink)".into();
        opts.count_name = "samples".into();
        opts.min_width = 0.1;

        let mut svg = Vec::new();
        flamegraph::from_reader(
            &mut opts,
            BufReader::new(Cursor::new(&diff_output)),
            &mut svg,
        )
        .expect("render diff flamegraph");

        let diff_path = "docs/flamecharts/flamechart_diff.svg";
        std::fs::write(diff_path, &svg).expect("write diff svg");
        let svg_str = String::from_utf8_lossy(&svg);
        verify_svg(&svg_str, diff_path);
    }

    // ── Allocation + latency measurement ───────────────────────────────
    measure_all();

    eprintln!("\nDone. Flamecharts in docs/flamecharts/");
}

/// Parse the generated SVG and print a summary of flame frames for verification.
fn verify_svg(svg: &str, path: &str) {
    let mut frames = Vec::new();
    let mut pos = 0;
    while let Some(start) = svg[pos..].find("<title>") {
        let start = pos + start + 7;
        if let Some(end) = svg[start..].find("</title>") {
            let title = &svg[start..start + end];
            if !title.is_empty() && title != "all" {
                frames.push(title.to_string());
            }
            pos = start + end + 8;
        } else {
            break;
        }
    }

    let rect_count = svg.matches("<rect ").count();
    eprintln!(
        "  Written: {} ({} bytes, {} rects, {} frames)",
        path,
        svg.len(),
        rect_count,
        frames.len()
    );
    eprintln!("  Top frames:");
    let mut parsed: Vec<(&str, u64)> = frames
        .iter()
        .filter_map(|f| {
            let paren = f.rfind('(')?;
            let name = f[..paren].trim();
            let inside = &f[paren + 1..f.rfind(')')?];
            let samples_str = inside.split(',').next()?.trim();
            let n: u64 = samples_str.split_whitespace().next()?.parse().ok()?;
            Some((name, n))
        })
        .collect();
    parsed.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, samples) in parsed.iter().take(8) {
        eprintln!("    {:>5} samples  {}", samples, name);
    }
}

/// Strip thread descriptors and binary-hash prefixes from collapsed stack lines.
fn clean_frames(collapsed: &str) -> String {
    collapsed
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let (frames_str, count) = line.rsplit_once(' ')?;

            let cleaned: Vec<String> = frames_str
                .split(';')
                .skip_while(|f| f.contains("Thread_") || f.contains("DispatchQueue"))
                .filter(|f| {
                    !f.ends_with("`start")
                        && !f.contains("lang_start")
                        && !f.contains("__rust_begin_short_backtrace")
                })
                .map(clean_one_frame)
                .collect();

            if cleaned.is_empty() {
                return None;
            }
            Some(format!("{} {}", cleaned.join(";"), count))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clean_one_frame(frame: &str) -> String {
    if let Some(func) = frame.strip_prefix("DYLD-STUB$$") {
        return func.to_string();
    }

    if let Some(pos) = frame.find('`') {
        let lib = &frame[..pos];
        let func = &frame[pos + 1..];
        if lib.starts_with("generate_flamecharts")
            || lib.starts_with("comparison_otel")
            || lib.starts_with("ro11y")
        {
            if func.contains("___rdl_alloc") {
                return "alloc::__rdl_alloc".into();
            }
            if func.contains("___rdl_dealloc") {
                return "alloc::__rdl_dealloc".into();
            }
            if func.contains("__rust_no_alloc_shim") {
                return "alloc::shim".into();
            }
            return func.to_string();
        }
    }

    frame.to_string()
}
