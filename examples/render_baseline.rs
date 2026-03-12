//! Reads `benches/baseline.toml` and renders `docs/performance.svg`.
//!
//! Usage: `cargo run --example render_baseline`

use std::collections::BTreeMap;

use plotters::prelude::*;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// TOML schema
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Baseline {
    metadata: Metadata,
    benchmarks: BTreeMap<String, Benchmark>,
    traffic_weights: BTreeMap<String, f64>,
    cpu_budget: CpuBudget,
}

#[derive(Deserialize)]
struct Metadata {
    version: String,
    date: String,
    platform: String,
    cpu: String,
    #[allow(dead_code)]
    rust_version: String,
    #[allow(dead_code)]
    notes: String,
}

#[derive(Deserialize)]
struct Benchmark {
    description: String,
    mean_ns: u64,
    #[allow(dead_code)]
    median_ns: u64,
    #[allow(dead_code)]
    allocations_approx: u64,
}

#[derive(Deserialize)]
struct CpuBudget {
    target_rps: u64,
    budget_percent: f64,
    budget_us_per_request: f64,
    #[allow(dead_code)]
    weighted_avg_us: f64,
    #[allow(dead_code)]
    total_cpu_percent: f64,
    #[allow(dead_code)]
    verdict: String,
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const SVG_W: u32 = 1200;
const SVG_H: u32 = 1600;
const BUDGET_US: f64 = 33.3;

fn short_name(name: &str) -> &str {
    name.strip_prefix("ecommerce_").unwrap_or(name)
}

fn bar_color(us: f64) -> RGBColor {
    let ratio = us / BUDGET_US;
    if ratio < 0.7 {
        RGBColor(76, 175, 80) // green
    } else if ratio < 1.0 {
        RGBColor(255, 193, 7) // yellow
    } else {
        RGBColor(244, 67, 54) // red
    }
}

fn main() {
    let toml_str = std::fs::read_to_string("benches/baseline.toml")
        .expect("Failed to read benches/baseline.toml — run from repo root");
    let baseline: Baseline = toml::from_str(&toml_str).expect("Failed to parse baseline TOML");

    // Collect ordered data
    let endpoint_order = [
        "ecommerce_catalog_browse",
        "ecommerce_search",
        "ecommerce_product_detail",
        "ecommerce_cart_add",
        "ecommerce_checkout",
        "ecommerce_health",
    ];
    let entries: Vec<(&str, &Benchmark, f64)> = endpoint_order
        .iter()
        .filter_map(|name| {
            let bench = baseline.benchmarks.get(*name)?;
            let weight = baseline.traffic_weights.get(*name).copied().unwrap_or(0.0);
            Some((*name, bench, weight))
        })
        .collect();

    let all_zero = entries.iter().all(|(_, b, _)| b.mean_ns == 0);

    // Compute derived values
    let weighted_avg_us: f64 = entries
        .iter()
        .map(|(_, b, w)| (b.mean_ns as f64 / 1000.0) * w)
        .sum();
    let total_cpu_pct =
        (weighted_avg_us * baseline.cpu_budget.target_rps as f64) / 1_000_000.0 * 100.0;
    let verdict = if all_zero {
        "PENDING"
    } else if total_cpu_pct <= baseline.cpu_budget.budget_percent {
        "PASS"
    } else {
        "FAIL"
    };

    // Write computed values back to baseline.toml (preserves comments/formatting)
    {
        let mut updated = toml_str.clone();
        for (key, val) in [
            ("weighted_avg_us", format!("{:.3}", weighted_avg_us)),
            ("total_cpu_percent", format!("{:.2}", total_cpu_pct)),
            ("verdict", format!("\"{}\"", verdict)),
        ] {
            let prefix = format!("{} = ", key);
            if let Some(start) = updated.find(&prefix) {
                let val_start = start + prefix.len();
                let val_end = updated[val_start..]
                    .find('\n')
                    .map(|i| val_start + i)
                    .unwrap_or(updated.len());
                updated.replace_range(val_start..val_end, &val);
            }
        }
        std::fs::write("benches/baseline.toml", &updated)
            .expect("Failed to write benches/baseline.toml");
        println!(
            "Updated baseline.toml: weighted_avg_us={:.3}, total_cpu_percent={:.2}, verdict={}",
            weighted_avg_us, total_cpu_pct, verdict
        );
    }

    // Create SVG
    let root = SVGBackend::new("docs/performance.svg", (SVG_W, SVG_H)).into_drawing_area();
    root.fill(&WHITE).unwrap();

    let title = format!(
        "ro11y v{} Performance Baseline — {} / {}",
        baseline.metadata.version, baseline.metadata.platform, baseline.metadata.cpu
    );
    root.draw(&Text::new(
        title,
        (SVG_W as i32 / 2, 25),
        ("sans-serif", 22).into_font().color(&BLACK),
    ))
    .unwrap();
    root.draw(&Text::new(
        format!("Date: {}", baseline.metadata.date),
        (SVG_W as i32 / 2, 50),
        ("sans-serif", 14)
            .into_font()
            .color(&RGBColor(100, 100, 100)),
    ))
    .unwrap();

    // Split into 4 panels
    let panels = root.margin(70, 20, 20, 20).split_evenly((4, 1));

    // -----------------------------------------------------------------------
    // Panel 1: Per-Operation Latency (horizontal bar chart)
    // -----------------------------------------------------------------------
    {
        let panel = &panels[0];
        let n = entries.len();
        let max_us = entries
            .iter()
            .map(|(_, b, _)| b.mean_ns as f64 / 1000.0)
            .fold(BUDGET_US * 1.5, f64::max);

        let mut chart = ChartBuilder::on(panel)
            .caption("Per-Operation Latency (us)", ("sans-serif", 18))
            .margin(10)
            .x_label_area_size(30)
            .y_label_area_size(160)
            .build_cartesian_2d(0.0..max_us, 0..n)
            .unwrap();

        chart
            .configure_mesh()
            .disable_y_mesh()
            .x_desc("Microseconds")
            .y_labels(n)
            .y_label_formatter(&|y| {
                entries
                    .get(*y)
                    .map(|(name, _, _)| short_name(name).to_string())
                    .unwrap_or_default()
            })
            .draw()
            .unwrap();

        // Budget threshold line
        chart
            .draw_series(LineSeries::new(
                vec![(BUDGET_US, 0), (BUDGET_US, n)],
                ShapeStyle::from(RGBColor(244, 67, 54)).stroke_width(2),
            ))
            .unwrap()
            .label(format!("{:.1}us budget", BUDGET_US))
            .legend(|(x, y)| {
                Rectangle::new(
                    [(x, y - 5), (x + 15, y + 5)],
                    RGBColor(244, 67, 54).filled(),
                )
            });

        // Bars
        for (i, (_, bench, _)) in entries.iter().enumerate() {
            let us = bench.mean_ns as f64 / 1000.0;
            let color = bar_color(us);
            chart
                .draw_series(std::iter::once(Rectangle::new(
                    [(0.0, i), (us, i + 1)],
                    color.mix(0.8).filled(),
                )))
                .unwrap();
            // Value label
            if bench.mean_ns > 0 {
                chart
                    .draw_series(std::iter::once(Text::new(
                        format!("{:.1}us", us),
                        (us + max_us * 0.02, i),
                        ("sans-serif", 12).into_font(),
                    )))
                    .unwrap();
            }
        }

        chart
            .configure_series_labels()
            .position(SeriesLabelPosition::UpperRight)
            .background_style(WHITE.mix(0.8))
            .draw()
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Panel 2: Throughput Capacity (vertical bar chart)
    // -----------------------------------------------------------------------
    {
        let panel = &panels[1];
        let n = entries.len();
        let capacities: Vec<f64> = entries
            .iter()
            .map(|(_, b, _)| {
                if b.mean_ns > 0 {
                    1e9 / b.mean_ns as f64
                } else {
                    0.0
                }
            })
            .collect();
        let max_cap = capacities
            .iter()
            .fold(baseline.cpu_budget.target_rps as f64 * 2.0, |a, &b| {
                a.max(b)
            });

        let mut chart = ChartBuilder::on(panel)
            .caption(
                "Throughput Capacity (ops/sec per endpoint)",
                ("sans-serif", 18),
            )
            .margin(10)
            .x_label_area_size(50)
            .y_label_area_size(160)
            .build_cartesian_2d(0..n, 0.0..max_cap)
            .unwrap();

        chart
            .configure_mesh()
            .disable_x_mesh()
            .y_desc("ops/sec")
            .x_labels(n)
            .x_label_formatter(&|x| {
                entries
                    .get(*x)
                    .map(|(name, _, _)| short_name(name).to_string())
                    .unwrap_or_default()
            })
            .draw()
            .unwrap();

        // Target line
        let target = baseline.cpu_budget.target_rps as f64;
        chart
            .draw_series(LineSeries::new(
                vec![(0, target), (n, target)],
                ShapeStyle::from(RGBColor(244, 67, 54)).stroke_width(2),
            ))
            .unwrap()
            .label(format!("{} req/s target", baseline.cpu_budget.target_rps))
            .legend(|(x, y)| {
                Rectangle::new(
                    [(x, y - 5), (x + 15, y + 5)],
                    RGBColor(244, 67, 54).filled(),
                )
            });

        // Bars
        for (i, cap) in capacities.iter().enumerate() {
            let color = RGBColor(33, 150, 243); // blue
            chart
                .draw_series(std::iter::once(Rectangle::new(
                    [(i, 0.0), (i + 1, *cap)],
                    color.mix(0.8).filled(),
                )))
                .unwrap();
        }

        chart
            .configure_series_labels()
            .position(SeriesLabelPosition::UpperRight)
            .background_style(WHITE.mix(0.8))
            .draw()
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Panel 3: CPU Budget at 3000 req/s (horizontal stacked bar)
    // -----------------------------------------------------------------------
    {
        let panel = &panels[2];
        let budget_pct = baseline.cpu_budget.budget_percent;

        // Each endpoint's CPU% contribution = (mean_us * weight * target_rps) / 1e6 * 100
        let mut contributions: Vec<(&str, f64)> = entries
            .iter()
            .map(|(name, bench, weight)| {
                let us = bench.mean_ns as f64 / 1000.0;
                let pct =
                    (us * weight * baseline.cpu_budget.target_rps as f64) / 1_000_000.0 * 100.0;
                (*name, pct)
            })
            .collect();
        contributions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        let total_obs_pct: f64 = contributions.iter().map(|(_, p)| p).sum();
        let max_x = (budget_pct * 1.5).max(total_obs_pct * 1.3);

        let mut chart = ChartBuilder::on(panel)
            .caption(
                format!(
                    "CPU Budget at {} req/s (% of one core)",
                    baseline.cpu_budget.target_rps
                ),
                ("sans-serif", 18),
            )
            .margin(10)
            .x_label_area_size(30)
            .y_label_area_size(160)
            .build_cartesian_2d(0.0..max_x, 0..3)
            .unwrap();

        chart
            .configure_mesh()
            .disable_y_mesh()
            .x_desc("CPU %")
            .y_labels(3)
            .y_label_formatter(&|y| match y {
                0 => "Observability".to_string(),
                1 => "Budget".to_string(),
                _ => String::new(),
            })
            .draw()
            .unwrap();

        // Budget line
        chart
            .draw_series(LineSeries::new(
                vec![(budget_pct, 0), (budget_pct, 3)],
                ShapeStyle::from(RGBColor(244, 67, 54)).stroke_width(2),
            ))
            .unwrap()
            .label(format!("{}% budget", budget_pct))
            .legend(|(x, y)| {
                Rectangle::new(
                    [(x, y - 5), (x + 15, y + 5)],
                    RGBColor(244, 67, 54).filled(),
                )
            });

        // Stacked segments for observability
        let colors = [
            RGBColor(33, 150, 243),
            RGBColor(76, 175, 80),
            RGBColor(255, 193, 7),
            RGBColor(156, 39, 176),
            RGBColor(255, 87, 34),
        ];
        let mut offset = 0.0;
        for (i, (name, pct)) in contributions.iter().enumerate() {
            let color = colors[i % colors.len()];
            chart
                .draw_series(std::iter::once(Rectangle::new(
                    [(offset, 0), (offset + pct, 1)],
                    color.mix(0.8).filled(),
                )))
                .unwrap()
                .label(format!("{}: {:.2}%", short_name(name), pct))
                .legend(move |(x, y)| {
                    Rectangle::new([(x, y - 5), (x + 15, y + 5)], color.filled())
                });
            offset += pct;
        }

        // Budget bar (gray)
        chart
            .draw_series(std::iter::once(Rectangle::new(
                [(0.0, 1), (budget_pct, 2)],
                RGBColor(200, 200, 200).mix(0.6).filled(),
            )))
            .unwrap();

        chart
            .configure_series_labels()
            .position(SeriesLabelPosition::UpperRight)
            .background_style(WHITE.mix(0.8))
            .draw()
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // Panel 4: Summary text
    // -----------------------------------------------------------------------
    {
        let panel = &panels[3];

        let lines = vec![
            format!("=== Summary ==="),
            format!(""),
            format!(
                "Weighted average overhead per request: {:.2} us",
                weighted_avg_us
            ),
            format!(
                "Total CPU at {} req/s: {:.2}%",
                baseline.cpu_budget.target_rps, total_cpu_pct
            ),
            format!(
                "Budget: {:.1}% (= {:.1} us/req)",
                baseline.cpu_budget.budget_percent, baseline.cpu_budget.budget_us_per_request
            ),
            format!("Verdict: {}", verdict),
            format!(""),
            format!("Per-endpoint breakdown:"),
        ];

        let mut y = 20;
        for line in &lines {
            panel
                .draw(&Text::new(
                    line.clone(),
                    (30, y),
                    ("monospace", 14).into_font().color(&BLACK),
                ))
                .unwrap();
            y += 20;
        }

        for (name, bench, weight) in &entries {
            let us = bench.mean_ns as f64 / 1000.0;
            let line = format!(
                "  {:24} {:>8.1} us  weight={:.0}%  desc={}",
                short_name(name),
                us,
                weight * 100.0,
                bench.description
            );
            panel
                .draw(&Text::new(
                    line,
                    (30, y),
                    ("monospace", 12).into_font().color(&RGBColor(60, 60, 60)),
                ))
                .unwrap();
            y += 18;
        }

        if all_zero {
            y += 10;
            panel
                .draw(&Text::new(
                    "NOTE: All values are 0 — run benchmarks and update baseline.toml".to_string(),
                    (30, y),
                    ("monospace", 14).into_font().color(&RGBColor(244, 67, 54)),
                ))
                .unwrap();
        }
    }

    root.present().unwrap();
    println!("Rendered docs/performance.svg");
}
