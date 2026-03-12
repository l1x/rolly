use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use ro11y::bench::*;
use tracing_subscriber::layer::SubscriberExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct EcommerceMetrics {
    registry: MetricsRegistry,
    // HTTP
    req_count: Counter,
    req_errors: Counter,
    active_requests: Gauge,
    req_duration: Histogram,
    req_body_size: Histogram,
    // Business
    items_viewed: Counter,
    search_queries: Counter,
    cart_items_added: Counter,
    cart_items_removed: Counter,
    cart_abandonment: Counter,
    revenue: Counter,
    payment_attempts: Counter,
    inventory_checks: Counter,
}

impl EcommerceMetrics {
    fn new() -> Self {
        let registry = MetricsRegistry::new();
        let latency_buckets = &[
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ];
        Self {
            req_count: registry.counter("http.server.requests", "Total HTTP requests"),
            req_errors: registry.counter("http.server.errors", "Total HTTP errors"),
            active_requests: registry.gauge("http.server.active_requests", "In-flight requests"),
            req_duration: registry.histogram(
                "http.server.duration",
                "Request latency in seconds",
                latency_buckets,
            ),
            req_body_size: registry.histogram(
                "http.server.request.body.size",
                "Request body bytes",
                &[64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0],
            ),
            items_viewed: registry.counter("ecommerce.items.viewed", "Items viewed"),
            search_queries: registry.counter("ecommerce.search.queries", "Search queries"),
            cart_items_added: registry.counter("ecommerce.cart.items_added", "Items added to cart"),
            cart_items_removed: registry
                .counter("ecommerce.cart.items_removed", "Items removed from cart"),
            cart_abandonment: registry.counter("ecommerce.cart.abandonments", "Cart abandonments"),
            revenue: registry.counter("ecommerce.revenue.cents", "Revenue in cents"),
            payment_attempts: registry.counter("ecommerce.payment.attempts", "Payment attempts"),
            inventory_checks: registry.counter("ecommerce.inventory.checks", "Inventory checks"),
            registry,
        }
    }
}

/// Create an OtlpLayer + Exporter with large capacity for benchmarks.
fn bench_subscriber() -> (
    impl tracing::Subscriber,
    tokio::sync::mpsc::Receiver<ExportMessage>,
) {
    let (exporter, rx) = Exporter::start_test_with_capacity(1_000_000);
    let layer = OtlpLayer::new(
        exporter,
        "bench-ecommerce",
        "0.5.1",
        "bench",
        true,
        true,
        1.0,
    );
    let subscriber = tracing_subscriber::registry().with(layer);
    (subscriber, rx)
}

/// A fixed trace ID for deterministic benchmarks.
fn trace_id_hex() -> String {
    let tid = generate_trace_id(Some("bench-request-id-001"));
    hex_encode(&tid)
}

// ---------------------------------------------------------------------------
// Endpoint simulations
//
// Each endpoint uses realistic attribute cardinality:
//   Spans:   6-10 attrs (http.*, user.*, server.*, session.*)
//   Metrics: 4-6 dims   (method, route, status, region, customer_tier, ...)
//   Logs:    3-6 fields  (structured business context)
// ---------------------------------------------------------------------------

#[inline(never)]
fn simulate_catalog_browse(metrics: &EcommerceMetrics, trace_id: &str) {
    let span = tracing::info_span!(
        "GET /products",
        trace_id = trace_id,
        http.method = "GET",
        http.route = "/products",
        http.status_code = 200i64,
        http.response.body.size = 12840i64,
        user.id = "usr_8f3a2b",
        user.tier = "premium",
        server.region = "eu-west-1",
        session.id = "sess_k9x2m",
    );
    let _enter = span.enter();

    tracing::info!(
        page = 1i64,
        per_page = 20i64,
        category = "electronics",
        sort_by = "popularity",
        cache_hit = true,
        result_count = 20i64,
        "catalog browse"
    );

    metrics
        .active_requests
        .set(42.0, &[("region", "eu-west-1")]);
    metrics.req_count.add(
        1,
        &[
            ("method", "GET"),
            ("route", "/products"),
            ("status", "200"),
            ("region", "eu-west-1"),
            ("customer_tier", "premium"),
        ],
    );
    metrics.items_viewed.add(
        20,
        &[
            ("page", "catalog"),
            ("category", "electronics"),
            ("region", "eu-west-1"),
        ],
    );
    metrics.req_duration.observe(
        0.012,
        &[
            ("method", "GET"),
            ("route", "/products"),
            ("region", "eu-west-1"),
            ("customer_tier", "premium"),
        ],
    );
    metrics
        .req_body_size
        .observe(0.0, &[("method", "GET"), ("route", "/products")]);
    // ~2% of browse requests fail (upstream timeout)
    metrics.req_errors.add(
        0,
        &[
            ("method", "GET"),
            ("route", "/products"),
            ("error_type", "none"),
            ("region", "eu-west-1"),
        ],
    );
}

#[inline(never)]
fn simulate_product_detail(metrics: &EcommerceMetrics, trace_id: &str) {
    let span = tracing::info_span!(
        "GET /products/:id",
        trace_id = trace_id,
        http.method = "GET",
        http.route = "/products/:id",
        http.status_code = 200i64,
        http.response.body.size = 4320i64,
        user.id = "usr_8f3a2b",
        user.tier = "member",
        server.region = "us-east-1",
        session.id = "sess_k9x2m",
    );
    let _enter = span.enter();

    tracing::info!(
        product_id = "SKU-42",
        product_category = "electronics",
        price_cents = 2999i64,
        currency = "USD",
        in_stock = true,
        "product detail view"
    );

    metrics.req_count.add(
        1,
        &[
            ("method", "GET"),
            ("route", "/products/:id"),
            ("status", "200"),
            ("region", "us-east-1"),
            ("customer_tier", "member"),
        ],
    );
    metrics.items_viewed.add(
        1,
        &[
            ("page", "detail"),
            ("category", "electronics"),
            ("region", "us-east-1"),
        ],
    );
    metrics.req_duration.observe(
        0.008,
        &[
            ("method", "GET"),
            ("route", "/products/:id"),
            ("region", "us-east-1"),
            ("customer_tier", "member"),
        ],
    );
}

#[inline(never)]
fn simulate_search(metrics: &EcommerceMetrics, trace_id: &str) {
    let span = tracing::info_span!(
        "GET /search",
        trace_id = trace_id,
        http.method = "GET",
        http.route = "/search",
        http.status_code = 200i64,
        http.response.body.size = 18200i64,
        user.id = "usr_8f3a2b",
        user.tier = "guest",
        server.region = "eu-west-1",
        session.id = "sess_k9x2m",
    );
    let _enter = span.enter();

    tracing::info!(
        query = "wireless headphones",
        filters = "brand:sony,price:50-200",
        result_count = 47i64,
        page = 1i64,
        facets_returned = 8i64,
        cache_hit = false,
        "search executed"
    );

    metrics.search_queries.add(
        1,
        &[
            ("region", "eu-west-1"),
            ("customer_tier", "guest"),
            ("cache_hit", "false"),
            ("has_filters", "true"),
        ],
    );
    metrics.req_count.add(
        1,
        &[
            ("method", "GET"),
            ("route", "/search"),
            ("status", "200"),
            ("region", "eu-west-1"),
            ("customer_tier", "guest"),
        ],
    );
    metrics.items_viewed.add(
        47,
        &[
            ("page", "search"),
            ("category", "mixed"),
            ("region", "eu-west-1"),
        ],
    );
    metrics.req_duration.observe(
        0.045,
        &[
            ("method", "GET"),
            ("route", "/search"),
            ("region", "eu-west-1"),
            ("customer_tier", "guest"),
        ],
    );
}

#[inline(never)]
fn simulate_cart_add(metrics: &EcommerceMetrics, trace_id: &str) {
    let span = tracing::info_span!(
        "POST /cart/add",
        trace_id = trace_id,
        http.method = "POST",
        http.route = "/cart/add",
        http.status_code = 200i64,
        http.request.body.size = 128i64,
        http.response.body.size = 512i64,
        user.id = "usr_8f3a2b",
        user.tier = "premium",
        server.region = "us-east-1",
        session.id = "sess_k9x2m",
    );
    let _enter = span.enter();

    tracing::info!(
        product_id = "SKU-42",
        product_category = "electronics",
        quantity = 1i64,
        unit_price_cents = 2999i64,
        currency = "USD",
        "item added to cart"
    );
    tracing::info!(
        cart_id = "cart_a1b2c3",
        cart_size = 3i64,
        cart_total_cents = 8997i64,
        currency = "USD",
        "cart updated"
    );

    metrics.req_count.add(
        1,
        &[
            ("method", "POST"),
            ("route", "/cart/add"),
            ("status", "200"),
            ("region", "us-east-1"),
            ("customer_tier", "premium"),
        ],
    );
    metrics.cart_items_added.add(
        1,
        &[
            ("product_category", "electronics"),
            ("region", "us-east-1"),
            ("currency", "USD"),
        ],
    );
    metrics
        .cart_abandonment
        .add(0, &[("region", "us-east-1"), ("customer_tier", "premium")]);
    metrics.req_duration.observe(
        0.015,
        &[
            ("method", "POST"),
            ("route", "/cart/add"),
            ("region", "us-east-1"),
            ("customer_tier", "premium"),
        ],
    );
    metrics
        .req_body_size
        .observe(128.0, &[("method", "POST"), ("route", "/cart/add")]);
}

#[inline(never)]
fn simulate_checkout(metrics: &EcommerceMetrics, trace_id: &str) {
    let span = tracing::info_span!(
        "POST /checkout",
        trace_id = trace_id,
        http.method = "POST",
        http.route = "/checkout",
        http.status_code = 200i64,
        http.request.body.size = 2048i64,
        http.response.body.size = 1024i64,
        user.id = "usr_8f3a2b",
        user.tier = "premium",
        server.region = "eu-west-1",
        session.id = "sess_k9x2m",
        cart.id = "cart_a1b2c3",
    );
    let _enter = span.enter();

    tracing::info!(
        order_id = "ORD-1234",
        items = 3i64,
        subtotal_cents = 14997i64,
        tax_cents = 1200i64,
        total_cents = 16197i64,
        currency = "EUR",
        shipping_method = "express",
        "checkout started"
    );

    // Child span: payment
    {
        let payment_span = tracing::info_span!(
            "payment_processing",
            payment.provider = "stripe",
            payment.method = "card",
            payment.currency = "EUR",
        );
        let _p = payment_span.enter();
        tracing::info!(
            provider = "stripe",
            amount_cents = 16197i64,
            currency = "EUR",
            card_brand = "visa",
            risk_score = 12i64,
            "payment processed"
        );
        metrics.payment_attempts.add(
            1,
            &[
                ("provider", "stripe"),
                ("status", "success"),
                ("currency", "EUR"),
                ("region", "eu-west-1"),
            ],
        );
        metrics.revenue.add(
            16197,
            &[
                ("currency", "EUR"),
                ("region", "eu-west-1"),
                ("payment_provider", "stripe"),
            ],
        );
    }

    // Child span: inventory reservation
    {
        let inv_span = tracing::info_span!(
            "inventory_check",
            warehouse.id = "wh-eu-01",
            warehouse.region = "eu-west-1",
        );
        let _i = inv_span.enter();
        tracing::info!(
            items_checked = 3i64,
            items_reserved = 3i64,
            warehouse = "wh-eu-01",
            "inventory verified and reserved"
        );
        metrics.inventory_checks.add(
            3,
            &[
                ("result", "in_stock"),
                ("warehouse", "wh-eu-01"),
                ("region", "eu-west-1"),
            ],
        );
    }

    // Child span: shipping label
    {
        let ship_span = tracing::info_span!(
            "shipping_label",
            carrier = "dhl",
            shipping.method = "express",
        );
        let _s = ship_span.enter();
        tracing::info!(
            carrier = "dhl",
            tracking_id = "DHL-EU-789456",
            estimated_days = 2i64,
            "shipping label created"
        );
    }

    metrics.req_count.add(
        1,
        &[
            ("method", "POST"),
            ("route", "/checkout"),
            ("status", "200"),
            ("region", "eu-west-1"),
            ("customer_tier", "premium"),
        ],
    );
    metrics.req_duration.observe(
        0.085,
        &[
            ("method", "POST"),
            ("route", "/checkout"),
            ("region", "eu-west-1"),
            ("customer_tier", "premium"),
        ],
    );
    metrics
        .req_body_size
        .observe(2048.0, &[("method", "POST"), ("route", "/checkout")]);
}

#[inline(never)]
fn simulate_health(trace_id: &str) {
    let span = tracing::info_span!(
        "GET /health",
        trace_id = trace_id,
        http.method = "GET",
        http.route = "/health",
        http.status_code = 200i64,
        server.region = "eu-west-1",
    );
    let _enter = span.enter();
    // No logs, no metrics — just the span
}

/// Run a mixed batch of N requests at weighted distribution.
/// Distribution: browse 40%, search 10%, detail 20%, cart_add 15%, checkout 8%,
///               cart_remove 3% (reuses cart_add path), health 4%
#[inline(never)]
fn simulate_mixed_batch(metrics: &EcommerceMetrics, trace_id: &str, n: usize) {
    for i in 0..n {
        let pct = i % 100;
        match pct {
            0..40 => simulate_catalog_browse(metrics, trace_id),
            40..50 => simulate_search(metrics, trace_id),
            50..70 => simulate_product_detail(metrics, trace_id),
            70..85 => simulate_cart_add(metrics, trace_id),
            85..93 => simulate_checkout(metrics, trace_id),
            93..96 => {
                // cart remove — similar cost to cart_add
                simulate_cart_add(metrics, trace_id);
                metrics.cart_items_removed.add(
                    1,
                    &[
                        ("product_category", "electronics"),
                        ("region", "us-east-1"),
                        ("currency", "USD"),
                    ],
                );
            }
            _ => simulate_health(trace_id),
        }
    }
}

// ---------------------------------------------------------------------------
// Benchmark groups
// ---------------------------------------------------------------------------

fn ecommerce_catalog_browse(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecommerce_catalog_browse");
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();
    let tid = trace_id_hex();

    group.bench_function("full_request", |b| {
        b.iter(|| simulate_catalog_browse(black_box(&metrics), black_box(&tid)));
    });
    group.finish();
}

fn ecommerce_product_detail(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecommerce_product_detail");
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();
    let tid = trace_id_hex();

    group.bench_function("full_request", |b| {
        b.iter(|| simulate_product_detail(black_box(&metrics), black_box(&tid)));
    });
    group.finish();
}

fn ecommerce_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecommerce_search");
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();
    let tid = trace_id_hex();

    group.bench_function("full_request", |b| {
        b.iter(|| simulate_search(black_box(&metrics), black_box(&tid)));
    });
    group.finish();
}

fn ecommerce_cart_add(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecommerce_cart_add");
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();
    let tid = trace_id_hex();

    group.bench_function("full_request", |b| {
        b.iter(|| simulate_cart_add(black_box(&metrics), black_box(&tid)));
    });
    group.finish();
}

fn ecommerce_checkout(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecommerce_checkout");
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();
    let tid = trace_id_hex();

    group.bench_function("full_request", |b| {
        b.iter(|| simulate_checkout(black_box(&metrics), black_box(&tid)));
    });
    group.finish();
}

fn ecommerce_health(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecommerce_health");
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let tid = trace_id_hex();

    group.bench_function("full_request", |b| {
        b.iter(|| simulate_health(black_box(&tid)));
    });
    group.finish();
}

fn ecommerce_mixed_traffic(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecommerce_mixed_traffic");
    group.throughput(Throughput::Elements(100));
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();
    let tid = trace_id_hex();

    group.bench_function("100_requests", |b| {
        b.iter(|| simulate_mixed_batch(black_box(&metrics), black_box(&tid), 100));
    });
    group.finish();
}

fn sustained_throughput_3000(c: &mut Criterion) {
    let mut group = c.benchmark_group("sustained_throughput_3000");
    group.throughput(Throughput::Elements(3000));
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();
    let tid = trace_id_hex();

    group.bench_function("3000_requests", |b| {
        b.iter(|| simulate_mixed_batch(black_box(&metrics), black_box(&tid), 3000));
    });
    group.finish();
}

fn metric_recording_under_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("metric_recording_under_load");
    let (subscriber, _rx) = bench_subscriber();
    let _guard = tracing::subscriber::set_default(subscriber);
    let metrics = EcommerceMetrics::new();

    // Pre-warm: create 100 distinct attribute series
    for i in 0..100 {
        let route = format!("/api/v{}", i);
        metrics.req_count.add(
            1,
            &[
                ("method", "GET"),
                ("route", &route),
                ("status", "200"),
                ("region", "eu-west-1"),
                ("customer_tier", "member"),
            ],
        );
    }

    group.bench_function("counter_add_5dims", |b| {
        b.iter(|| {
            metrics.req_count.add(
                black_box(1),
                black_box(&[
                    ("method", "GET"),
                    ("route", "/products"),
                    ("status", "200"),
                    ("region", "eu-west-1"),
                    ("customer_tier", "premium"),
                ]),
            );
        });
    });

    group.bench_function("histogram_observe_4dims", |b| {
        b.iter(|| {
            metrics.req_duration.observe(
                black_box(0.042),
                black_box(&[
                    ("method", "GET"),
                    ("route", "/products"),
                    ("region", "eu-west-1"),
                    ("customer_tier", "premium"),
                ]),
            );
        });
    });

    group.bench_function("collect_100_series", |b| {
        b.iter(|| {
            black_box(metrics.registry.collect());
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    ecommerce_catalog_browse,
    ecommerce_product_detail,
    ecommerce_search,
    ecommerce_cart_add,
    ecommerce_checkout,
    ecommerce_health,
    ecommerce_mixed_traffic,
    sustained_throughput_3000,
    metric_recording_under_load,
);
criterion_main!(benches);
