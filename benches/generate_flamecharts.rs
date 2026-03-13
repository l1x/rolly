//! Generate flamechart SVGs and criterion-based comparison outputs.
//!
//! Produces in `docs/flamecharts/`:
//! - `flamechart_after.svg`   — optimized rolly Counter::add
//! - `flamechart_otel.svg`    — OpenTelemetry SDK 0.31 Counter::add
//! - `comparison_table.svg`   — criterion benchmark comparison with 95% CIs
//! - `benchmark_results.toml` — machine-readable criterion results
//!
//! Run: `cargo bench --features _bench --bench generate_flamecharts`
//!
//! Requires:
//! - macOS `sample` command (ships with Xcode CLI tools) for flamecharts
//! - Prior `cargo bench --features _bench -- comparison` run for criterion data

use std::fmt::Write;
use std::hint::black_box;
use std::io::{BufReader, Cursor};
use std::process::Command;
use std::time::{Duration, Instant};

// ── Constants ──────────────────────────────────────────────────────────

const PROFILE_SECS: u64 = 12;
const SAMPLE_DELAY_SECS: u64 = 3;
const SAMPLE_SECS: &str = "7";

// ── Main ───────────────────────────────────────────────────────────────

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--after") => profile_after(),
        Some("--otel") => profile_otel(),
        _ => generate_all(),
    }
}

// ── Profiling: optimized rolly ─────────────────────────────────────────

fn profile_after() {
    use rolly::bench::*;
    let registry = MetricsRegistry::new();
    let counter = registry.counter("test", "test");
    counter.add(
        1,
        &[
            ("method", "GET"),
            ("status", "200"),
            ("region", "us-east-1"),
        ],
    );

    let deadline = Instant::now() + Duration::from_secs(PROFILE_SECS);
    while Instant::now() < deadline {
        for _ in 0..1000 {
            counter.add(
                black_box(1),
                black_box(&[
                    ("method", "GET"),
                    ("status", "200"),
                    ("region", "us-east-1"),
                ]),
            );
        }
    }
}

// ── Profiling: OTel SDK (fair — pre-built attrs) ──────────────────────

fn profile_otel() {
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::metrics::{ManualReader, SdkMeterProvider};

    let provider = SdkMeterProvider::builder()
        .with_reader(ManualReader::builder().build())
        .build();
    let meter = provider.meter("bench");
    let ctr = meter.u64_counter("test").build();

    let attrs = vec![
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
    ];
    ctr.add(1, &attrs);

    let deadline = Instant::now() + Duration::from_secs(PROFILE_SECS);
    while Instant::now() < deadline {
        for _ in 0..1000 {
            ctr.add(black_box(1), black_box(&attrs));
        }
    }
}

// ── Criterion result types ─────────────────────────────────────────────

#[derive(Debug)]
struct BenchmarkResult {
    mean_ns: f64,
    ci_lower: f64,
    ci_upper: f64,
}

#[derive(Debug)]
struct BenchmarkGroup {
    name: String,
    rolly: Option<BenchmarkResult>,
    otel: Option<BenchmarkResult>,
}

// ── Read criterion JSON results ────────────────────────────────────────

fn read_criterion_results() -> Vec<BenchmarkGroup> {
    use serde_json::Value;

    let base = std::path::Path::new("target/criterion");
    if !base.exists() {
        eprintln!("  WARNING: target/criterion/ not found. Run `cargo bench --features _bench -- comparison` first.");
        return Vec::new();
    }

    let mut groups: std::collections::BTreeMap<String, BenchmarkGroup> =
        std::collections::BTreeMap::new();

    let entries: Vec<_> = std::fs::read_dir(base)
        .expect("read target/criterion")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("comparison_"))
        })
        .collect();

    for entry in entries {
        let group_name = entry.file_name().to_string_lossy().to_string();

        for variant_name in &["rolly", "otel_sdk"] {
            let estimates_path = entry
                .path()
                .join(variant_name)
                .join("new")
                .join("estimates.json");

            if !estimates_path.exists() {
                continue;
            }

            let data = match std::fs::read_to_string(&estimates_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("  WARNING: cannot read {}: {}", estimates_path.display(), e);
                    continue;
                }
            };

            let json: Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "  WARNING: cannot parse {}: {}",
                        estimates_path.display(),
                        e
                    );
                    continue;
                }
            };

            let mean = &json["mean"];
            let point_estimate = mean["point_estimate"].as_f64().unwrap_or(0.0);
            let ci = &mean["confidence_interval"];
            let lower = ci["lower_bound"].as_f64().unwrap_or(0.0);
            let upper = ci["upper_bound"].as_f64().unwrap_or(0.0);

            let result = BenchmarkResult {
                mean_ns: point_estimate,
                ci_lower: lower,
                ci_upper: upper,
            };

            let group = groups
                .entry(group_name.clone())
                .or_insert_with(|| BenchmarkGroup {
                    name: group_name.clone(),
                    rolly: None,
                    otel: None,
                });

            if *variant_name == "rolly" {
                group.rolly = Some(result);
            } else {
                group.otel = Some(result);
            }
        }
    }

    groups.into_values().collect()
}

/// Pretty-print a criterion group name: "comparison_counter_3_attrs" -> "Counter (3 attrs)"
fn pretty_group_name(raw: &str) -> String {
    let s = raw.strip_prefix("comparison_").unwrap_or(raw);

    // Split into parts: ["counter", "3", "attrs"] or ["counter", "no", "attrs"]
    let parts: Vec<&str> = s.split('_').collect();

    if parts.is_empty() {
        return raw.to_string();
    }

    // Capitalize metric type
    let metric_type = {
        let mut c = parts[0].chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().to_string() + c.as_str(),
        }
    };

    // Check for "cold" suffix
    let is_cold = parts.last() == Some(&"cold");
    let attr_parts = if is_cold {
        &parts[1..parts.len() - 1]
    } else {
        &parts[1..]
    };

    let attr_desc = attr_parts.join(" ");
    let cold_suffix = if is_cold { " cold" } else { "" };

    if attr_desc.is_empty() {
        format!("{}{}", metric_type, cold_suffix)
    } else {
        format!("{} ({}{})", metric_type, attr_desc, cold_suffix)
    }
}

// ── TOML output from criterion data ────────────────────────────────────

fn write_criterion_toml(groups: &[BenchmarkGroup]) {
    let mut t = String::with_capacity(2048);
    t.push_str("# Source: criterion benchmark results\n");
    t.push_str("# Run: cargo bench --features _bench -- comparison\n");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = write!(t, "generated_unix = {}\n\n", now);

    for g in groups {
        // Sanitize name for TOML key
        let key = g.name.strip_prefix("comparison_").unwrap_or(&g.name);

        let _ = writeln!(t, "[{}]", key);

        if let Some(ref r) = g.rolly {
            let _ = writeln!(t, "rolly_mean_ns = {:.1}", r.mean_ns);
            let _ = writeln!(t, "rolly_ci_lower = {:.1}", r.ci_lower);
            let _ = writeln!(t, "rolly_ci_upper = {:.1}", r.ci_upper);
        }

        if let Some(ref o) = g.otel {
            let _ = writeln!(t, "otel_mean_ns = {:.1}", o.mean_ns);
            let _ = writeln!(t, "otel_ci_lower = {:.1}", o.ci_lower);
            let _ = writeln!(t, "otel_ci_upper = {:.1}", o.ci_upper);
        }

        t.push('\n');
    }

    let path = "docs/flamecharts/benchmark_results.toml";
    std::fs::write(path, &t).expect("write benchmark_results.toml");
    eprintln!("  Written: {} ({} bytes)", path, t.len());
}

// ── SVG comparison table with CIs ──────────────────────────────────────

fn render_comparison_table_svg(groups: &[BenchmarkGroup]) {
    // Columns: Scenario | rolly (ns) | OTel SDK (ns) | Speedup
    let col_x: [f64; 5] = [20.0, 260.0, 430.0, 600.0, 740.0];
    let total_w = 760.0;
    let row_h = 40.0;
    let title_h = 70.0;
    let hdr_h = 36.0;
    let num_rows = groups.len() as f64;
    let table_top = title_h;
    let table_h = hdr_h + row_h * num_rows;
    let total_h = title_h + table_h + 30.0;

    let mut s = String::with_capacity(8192);

    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" \
         font-family=\"'Inter','SF Pro Display',system-ui,sans-serif\" font-size=\"14\">",
        total_w, total_h
    );
    s.push_str(concat!(
        "<style>\n",
        "  .title { font-size: 17px; font-weight: 700; fill: #111827; letter-spacing: -0.02em; }\n",
        "  .subtitle { font-size: 12px; fill: #9ca3af; }\n",
        "  .hdr { font-size: 12px; font-weight: 600; fill: #6b7280; }\n",
        "  .scenario { font-size: 14px; fill: #111827; font-weight: 500; }\n",
        "  .v { font-size: 13px; fill: #374151; text-anchor: end; font-variant-numeric: tabular-nums; }\n",
        "  .best { fill: #059669; font-weight: 600; }\n",
        "  .worst { fill: #dc2626; }\n",
        "  .neutral { fill: #6b7280; }\n",
        "  .grid { stroke: #e5e7eb; stroke-width: 1; }\n",
        "  .grid-thick { stroke: #d1d5db; stroke-width: 1.5; }\n",
        "</style>\n",
    ));

    // Background
    let _ = writeln!(
        s,
        "<rect width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>",
        total_w, total_h
    );

    // Title
    s.push_str(
        "<text x=\"20\" y=\"28\" class=\"title\">\
         rolly vs OpenTelemetry SDK 0.31 \u{2014} Criterion Benchmarks (95% CI)</text>\n",
    );
    s.push_str(
        "<text x=\"20\" y=\"48\" class=\"subtitle\">\
         Values show mean \u{00b1} CI half-width in nanoseconds. \
         Green = rolly faster, red = rolly slower.</text>\n",
    );

    // Header row background
    let _ = writeln!(
        s,
        "<rect x=\"0\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#f8fafc\"/>",
        table_top, total_w, hdr_h
    );

    // Header bottom border
    let hdr_line_y = table_top + hdr_h;
    let _ = writeln!(
        s,
        "<line x1=\"0\" y1=\"{}\" x2=\"{}\" y2=\"{}\" class=\"grid-thick\"/>",
        hdr_line_y, total_w, hdr_line_y
    );

    // Header labels
    let headers = [
        ("Scenario", "start"),
        ("rolly (ns)", "end"),
        ("OTel SDK (ns)", "end"),
        ("Speedup", "end"),
    ];
    let hdr_y = table_top + 23.0;
    for (i, (label, anchor)) in headers.iter().enumerate() {
        let hx = if *anchor == "start" {
            col_x[i] + 12.0
        } else {
            col_x[i + 1] - 12.0
        };
        let _ = writeln!(
            s,
            "<text x=\"{}\" y=\"{}\" class=\"hdr\" text-anchor=\"{}\">{}</text>",
            hx, hdr_y, anchor, label
        );
    }

    // Data rows
    for (idx, g) in groups.iter().enumerate() {
        let ry = table_top + hdr_h + (idx as f64) * row_h;

        // Alternating row background
        if idx % 2 == 1 {
            let _ = writeln!(
                s,
                "<rect x=\"0\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#f9fafb\"/>",
                ry, total_w, row_h
            );
        }

        // Row bottom border
        let _ = writeln!(
            s,
            "<line x1=\"0\" y1=\"{}\" x2=\"{}\" y2=\"{}\" class=\"grid\"/>",
            ry + row_h,
            total_w,
            ry + row_h
        );

        let ty = ry + 26.0;

        // Col 0: Scenario name
        let pretty = pretty_group_name(&g.name);
        let _ = writeln!(
            s,
            "<text x=\"{}\" y=\"{}\" class=\"scenario\">{}</text>",
            col_x[0] + 12.0,
            ty,
            pretty
        );

        // Col 1: rolly mean +/- CI
        if let Some(ref r) = g.rolly {
            let ci_half = (r.ci_upper - r.ci_lower) / 2.0;
            let _ = writeln!(
                s,
                "<text x=\"{}\" y=\"{}\" class=\"v\">{:.1} \u{00b1} {:.1}</text>",
                col_x[2] - 12.0,
                ty,
                r.mean_ns,
                ci_half
            );
        } else {
            let _ = writeln!(
                s,
                "<text x=\"{}\" y=\"{}\" class=\"v neutral\">\u{2014}</text>",
                col_x[2] - 12.0,
                ty
            );
        }

        // Col 2: OTel mean +/- CI
        if let Some(ref o) = g.otel {
            let ci_half = (o.ci_upper - o.ci_lower) / 2.0;
            let _ = writeln!(
                s,
                "<text x=\"{}\" y=\"{}\" class=\"v\">{:.1} \u{00b1} {:.1}</text>",
                col_x[3] - 12.0,
                ty,
                o.mean_ns,
                ci_half
            );
        } else {
            let _ = writeln!(
                s,
                "<text x=\"{}\" y=\"{}\" class=\"v neutral\">\u{2014}</text>",
                col_x[3] - 12.0,
                ty
            );
        }

        // Col 3: Speedup
        if let (Some(ref r), Some(ref o)) = (&g.rolly, &g.otel) {
            let speedup = o.mean_ns / r.mean_ns;
            let (class, label) = if speedup >= 1.05 {
                ("v best", format!("{:.1}x faster", speedup))
            } else if speedup <= 0.95 {
                ("v worst", format!("{:.1}x slower", 1.0 / speedup))
            } else {
                ("v neutral", "~parity".to_string())
            };
            let _ = writeln!(
                s,
                "<text x=\"{}\" y=\"{}\" class=\"{}\">{}</text>",
                col_x[4] - 12.0,
                ty,
                class,
                label
            );
        } else {
            let _ = writeln!(
                s,
                "<text x=\"{}\" y=\"{}\" class=\"v neutral\">\u{2014}</text>",
                col_x[4] - 12.0,
                ty
            );
        }
    }

    // Vertical column separators
    let grid_top = table_top;
    let grid_bot = table_top + table_h;
    for &cx in &col_x[1..] {
        let _ = writeln!(
            s,
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" class=\"grid\"/>",
            cx, grid_top, cx, grid_bot
        );
    }

    // Footer note
    let footer_y = table_top + table_h + 18.0;
    let _ = writeln!(
        s,
        "<text x=\"20\" y=\"{}\" class=\"subtitle\">\
         Source: criterion (cargo bench --features _bench -- comparison). \
         CI = 95% confidence interval on mean.</text>",
        footer_y
    );

    s.push_str("</svg>\n");

    let path = "docs/flamecharts/comparison_table.svg";
    std::fs::write(path, &s).expect("write comparison_table.svg");
    eprintln!("  Written: {} ({} bytes)", path, s.len());
}

// ── Flamechart generation ──────────────────────────────────────────────

fn generate_all() {
    use inferno::collapse::{sample, Collapse};
    use inferno::flamegraph;

    std::fs::create_dir_all("docs/flamecharts").expect("create docs/flamecharts");
    let exe = std::env::current_exe().expect("current_exe");

    let variants: &[(&str, &str, &str)] = &[
        (
            "--after",
            "flamechart_after.svg",
            "rolly Counter::add (3 attrs) \u{2014} Optimized",
        ),
        (
            "--otel",
            "flamechart_otel.svg",
            "OpenTelemetry SDK 0.31 Counter::add (3 attrs)",
        ),
    ];

    for (arg, filename, title) in variants {
        let svg_path = format!("docs/flamecharts/{}", filename);
        let tag = arg.trim_start_matches("--");
        let sample_path = format!("/tmp/rolly_flamechart_{}.txt", tag);

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

    // ── Read criterion results and generate comparison outputs ──────
    eprintln!("\n=== Criterion benchmark results ===");
    let groups = read_criterion_results();

    if groups.is_empty() {
        eprintln!("  No criterion results found. Skipping TOML + SVG generation.");
        eprintln!("  Run `cargo bench --features _bench -- comparison` first.");
    } else {
        for g in &groups {
            let pretty = pretty_group_name(&g.name);
            if let (Some(ref r), Some(ref o)) = (&g.rolly, &g.otel) {
                let speedup = o.mean_ns / r.mean_ns;
                let r_ci = (r.ci_upper - r.ci_lower) / 2.0;
                let o_ci = (o.ci_upper - o.ci_lower) / 2.0;
                eprintln!(
                    "  {:<30} rolly: {:.1} \u{00b1} {:.1} ns  OTel: {:.1} \u{00b1} {:.1} ns  ({:.1}x)",
                    pretty, r.mean_ns, r_ci, o.mean_ns, o_ci, speedup
                );
            } else {
                eprintln!("  {:<30} (incomplete data)", pretty);
            }
        }

        write_criterion_toml(&groups);
        render_comparison_table_svg(&groups);
    }

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
            || lib.starts_with("rolly")
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
