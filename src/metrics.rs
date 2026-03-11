use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// Global metrics registry.
static GLOBAL_REGISTRY: OnceLock<MetricsRegistry> = OnceLock::new();

/// Get or initialize the global registry.
pub fn global_registry() -> &'static MetricsRegistry {
    GLOBAL_REGISTRY.get_or_init(MetricsRegistry::new)
}

/// Snapshot of a single histogram data point.
pub struct HistogramDataPoint {
    pub attrs: Arc<Vec<(String, String)>>,
    pub bucket_counts: Vec<u64>,
    pub sum: f64,
    pub count: u64,
    pub min: f64,
    pub max: f64,
    pub exemplar: Option<Exemplar>,
}

/// An exemplar linking a metric data point to a trace.
#[derive(Clone, Debug)]
pub struct Exemplar {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub time_unix_nano: u64,
    pub value: ExemplarValue,
}

/// The measured value attached to an exemplar.
#[derive(Clone, Debug)]
pub enum ExemplarValue {
    Int(i64),
    Double(f64),
}

/// Sorted attribute pairs. Wrapped in `Arc` for zero-copy snapshots during `collect()`.
pub type Attrs = Arc<Vec<(String, String)>>;

/// A counter data point: (attrs, cumulative value, optional exemplar).
pub type CounterDataPoint = (Attrs, i64, Option<Exemplar>);

/// A gauge data point: (attrs, last value, optional exemplar).
pub type GaugeDataPoint = (Attrs, f64, Option<Exemplar>);

/// A snapshot of a single metric for encoding.
pub enum MetricSnapshot {
    Counter {
        name: String,
        description: String,
        data_points: Vec<CounterDataPoint>,
    },
    Gauge {
        name: String,
        description: String,
        data_points: Vec<GaugeDataPoint>,
    },
    Histogram {
        name: String,
        description: String,
        boundaries: Vec<f64>,
        data_points: Vec<HistogramDataPoint>,
    },
}

/// Central registry holding all counters, gauges, and histograms.
pub struct MetricsRegistry {
    counters: RwLock<HashMap<String, Counter>>,
    gauges: RwLock<HashMap<String, Gauge>>,
    histograms: RwLock<HashMap<String, Histogram>>,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            counters: RwLock::new(HashMap::new()),
            gauges: RwLock::new(HashMap::new()),
            histograms: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a counter by name.
    pub fn counter(&self, name: &str, description: &str) -> Counter {
        // Fast path: read lock
        {
            let counters = self.counters.read().unwrap();
            if let Some(c) = counters.get(name) {
                return c.clone();
            }
        }
        // Slow path: write lock
        let mut counters = self.counters.write().unwrap();
        counters
            .entry(name.to_string())
            .or_insert_with(|| Counter {
                inner: Arc::new(CounterInner {
                    name: name.to_string(),
                    description: description.to_string(),
                    data: Mutex::new(HashMap::new()),
                }),
            })
            .clone()
    }

    /// Get or create a gauge by name.
    pub fn gauge(&self, name: &str, description: &str) -> Gauge {
        // Fast path: read lock
        {
            let gauges = self.gauges.read().unwrap();
            if let Some(g) = gauges.get(name) {
                return g.clone();
            }
        }
        // Slow path: write lock
        let mut gauges = self.gauges.write().unwrap();
        gauges
            .entry(name.to_string())
            .or_insert_with(|| Gauge {
                inner: Arc::new(GaugeInner {
                    name: name.to_string(),
                    description: description.to_string(),
                    data: Mutex::new(HashMap::new()),
                }),
            })
            .clone()
    }

    /// Get or create a histogram by name.
    /// Boundaries are sorted and deduplicated at creation time.
    pub fn histogram(&self, name: &str, description: &str, boundaries: &[f64]) -> Histogram {
        // Fast path: read lock
        {
            let histograms = self.histograms.read().unwrap();
            if let Some(h) = histograms.get(name) {
                return h.clone();
            }
        }
        // Slow path: write lock
        let mut sorted = boundaries.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted.dedup();
        let mut histograms = self.histograms.write().unwrap();
        histograms
            .entry(name.to_string())
            .or_insert_with(|| Histogram {
                inner: Arc::new(HistogramInner {
                    name: name.to_string(),
                    description: description.to_string(),
                    boundaries: sorted,
                    data: Mutex::new(HashMap::new()),
                }),
            })
            .clone()
    }

    /// Snapshot all metrics for encoding. Does not reset counters (cumulative).
    /// Exemplars are consumed (reset to `None`) on each collect — fresh sample each interval.
    pub fn collect(&self) -> Vec<MetricSnapshot> {
        let mut snapshots = Vec::new();

        {
            let counters = self.counters.read().unwrap();
            for counter in counters.values() {
                let mut data = counter.inner.data.lock().unwrap();
                if data.is_empty() {
                    continue;
                }
                let data_points: Vec<_> = data
                    .values_mut()
                    .map(|(attrs, val, exemplar)| (Arc::clone(attrs), *val, exemplar.take()))
                    .collect();
                snapshots.push(MetricSnapshot::Counter {
                    name: counter.inner.name.clone(),
                    description: counter.inner.description.clone(),
                    data_points,
                });
            }
        }

        {
            let gauges = self.gauges.read().unwrap();
            for gauge in gauges.values() {
                let mut data = gauge.inner.data.lock().unwrap();
                if data.is_empty() {
                    continue;
                }
                let data_points: Vec<_> = data
                    .values_mut()
                    .map(|(attrs, val, exemplar)| (Arc::clone(attrs), *val, exemplar.take()))
                    .collect();
                snapshots.push(MetricSnapshot::Gauge {
                    name: gauge.inner.name.clone(),
                    description: gauge.inner.description.clone(),
                    data_points,
                });
            }
        }

        {
            let histograms = self.histograms.read().unwrap();
            for histogram in histograms.values() {
                let mut data = histogram.inner.data.lock().unwrap();
                if data.is_empty() {
                    continue;
                }
                let data_points: Vec<_> = data
                    .values_mut()
                    .map(|(attrs, state, exemplar)| HistogramDataPoint {
                        attrs: Arc::clone(attrs),
                        bucket_counts: state.bucket_counts.clone(),
                        sum: state.sum,
                        count: state.count,
                        min: state.min,
                        max: state.max,
                        exemplar: exemplar.take(),
                    })
                    .collect();
                snapshots.push(MetricSnapshot::Histogram {
                    name: histogram.inner.name.clone(),
                    description: histogram.inner.description.clone(),
                    boundaries: histogram.inner.boundaries.clone(),
                    data_points,
                });
            }
        }

        snapshots
    }
}

/// Compute a hash key for a sorted set of attribute pairs.
fn attrs_hash(attrs: &[(&str, &str)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for (k, v) in attrs {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }
    hasher.finish()
}

/// Sort and own attribute pairs, wrapped in Arc for zero-copy snapshots.
fn owned_attrs(attrs: &[(&str, &str)]) -> Arc<Vec<(String, String)>> {
    let mut owned: Vec<(String, String)> = attrs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    owned.sort();
    Arc::new(owned)
}

/// Read the current span's trace_id and span_id from the tracing subscriber.
/// Returns `None` when no span is active or no `SpanFields` extension is found.
fn current_trace_context() -> Option<([u8; 16], [u8; 8])> {
    let mut result = None;
    tracing::Span::current().with_subscriber(|(id, dispatch)| {
        use tracing_subscriber::registry::LookupSpan;
        if let Some(registry) = dispatch.downcast_ref::<tracing_subscriber::Registry>() {
            if let Some(span_ref) = registry.span(id) {
                let ext = span_ref.extensions();
                if let Some(fields) = ext.get::<crate::otlp_layer::SpanFields>() {
                    result = Some((fields.trace_id, fields.span_id));
                } else {
                    for ancestor in span_ref.scope().skip(1) {
                        let ext = ancestor.extensions();
                        if let Some(fields) = ext.get::<crate::otlp_layer::SpanFields>() {
                            result = Some((fields.trace_id, fields.span_id));
                            break;
                        }
                    }
                }
            }
        }
    });
    result
}

/// Capture an exemplar from the current trace context if available.
/// Skips all-zero trace_ids (no active trace).
fn capture_exemplar(value: ExemplarValue) -> Option<Exemplar> {
    let (trace_id, span_id) = current_trace_context()?;
    if trace_id == [0u8; 16] {
        return None;
    }
    let time_unix_nano = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    Some(Exemplar {
        trace_id,
        span_id,
        time_unix_nano,
        value,
    })
}

// --- Counter ---

struct CounterInner {
    name: String,
    description: String,
    data: Mutex<HashMap<u64, CounterDataPoint>>,
}

/// A monotonic u64 counter. Clone is cheap (Arc).
#[derive(Clone)]
pub struct Counter {
    inner: Arc<CounterInner>,
}

impl Counter {
    /// Add a value to the counter for the given attribute set.
    pub fn add(&self, value: u64, attrs: &[(&str, &str)]) {
        let exemplar = capture_exemplar(ExemplarValue::Int(value as i64));
        let mut sorted = attrs.to_vec();
        sorted.sort();
        let key = attrs_hash(&sorted);
        let mut data = self.inner.data.lock().unwrap();
        let entry = data
            .entry(key)
            .or_insert_with(|| (owned_attrs(&sorted), 0, None));
        entry.1 += value as i64;
        if exemplar.is_some() {
            entry.2 = exemplar;
        }
    }
}

// --- Gauge ---

struct GaugeInner {
    name: String,
    description: String,
    data: Mutex<HashMap<u64, GaugeDataPoint>>,
}

/// A last-value f64 gauge. Clone is cheap (Arc).
#[derive(Clone)]
pub struct Gauge {
    inner: Arc<GaugeInner>,
}

impl Gauge {
    /// Set the gauge to a value for the given attribute set.
    pub fn set(&self, value: f64, attrs: &[(&str, &str)]) {
        let exemplar = capture_exemplar(ExemplarValue::Double(value));
        let mut sorted = attrs.to_vec();
        sorted.sort();
        let key = attrs_hash(&sorted);
        let mut data = self.inner.data.lock().unwrap();
        let entry = data
            .entry(key)
            .or_insert_with(|| (owned_attrs(&sorted), 0.0, None));
        entry.1 = value;
        if exemplar.is_some() {
            entry.2 = exemplar;
        }
    }
}

// --- Histogram ---

struct HistogramState {
    bucket_counts: Vec<u64>,
    sum: f64,
    count: u64,
    min: f64,
    max: f64,
}

type HistogramEntry = (Attrs, HistogramState, Option<Exemplar>);

struct HistogramInner {
    name: String,
    description: String,
    boundaries: Vec<f64>,
    data: Mutex<HashMap<u64, HistogramEntry>>,
}

/// A histogram with client-side bucketing. Clone is cheap (Arc).
#[derive(Clone)]
pub struct Histogram {
    inner: Arc<HistogramInner>,
}

impl Histogram {
    /// Record an observed value for the given attribute set.
    pub fn observe(&self, value: f64, attrs: &[(&str, &str)]) {
        let exemplar = capture_exemplar(ExemplarValue::Double(value));
        let bucket_idx = self.inner.boundaries.partition_point(|&b| b <= value);
        let mut sorted = attrs.to_vec();
        sorted.sort();
        let key = attrs_hash(&sorted);
        let mut data = self.inner.data.lock().unwrap();
        let entry = data.entry(key).or_insert_with(|| {
            let num_buckets = self.inner.boundaries.len() + 1;
            (
                owned_attrs(&sorted),
                HistogramState {
                    bucket_counts: vec![0; num_buckets],
                    sum: 0.0,
                    count: 0,
                    min: f64::INFINITY,
                    max: f64::NEG_INFINITY,
                },
                None,
            )
        });
        entry.1.bucket_counts[bucket_idx] += 1;
        entry.1.sum += value;
        entry.1.count += 1;
        if value < entry.1.min {
            entry.1.min = value;
        }
        if value > entry.1.max {
            entry.1.max = value;
        }
        if exemplar.is_some() {
            entry.2 = exemplar;
        }
    }
}

// --- Public API ---

/// Get or create a named counter from the global registry.
pub fn counter(name: &str, description: &str) -> Counter {
    global_registry().counter(name, description)
}

/// Get or create a named gauge from the global registry.
pub fn gauge(name: &str, description: &str) -> Gauge {
    global_registry().gauge(name, description)
}

/// Get or create a named histogram from the global registry.
pub fn histogram(name: &str, description: &str, boundaries: &[f64]) -> Histogram {
    global_registry().histogram(name, description, boundaries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_add_accumulates() {
        let registry = MetricsRegistry::new();
        let c = registry.counter("req_total", "Total requests");
        c.add(1, &[("method", "GET")]);
        c.add(3, &[("method", "GET")]);
        c.add(1, &[("method", "POST")]);

        let snapshots = registry.collect();
        assert_eq!(snapshots.len(), 1);
        match &snapshots[0] {
            MetricSnapshot::Counter {
                name, data_points, ..
            } => {
                assert_eq!(name, "req_total");
                assert_eq!(data_points.len(), 2);
                let get_val = data_points
                    .iter()
                    .find(|(a, _, _)| a[0].1 == "GET")
                    .unwrap()
                    .1;
                assert_eq!(get_val, 4);
                let post_val = data_points
                    .iter()
                    .find(|(a, _, _)| a[0].1 == "POST")
                    .unwrap()
                    .1;
                assert_eq!(post_val, 1);
            }
            _ => panic!("expected Counter snapshot"),
        }
    }

    #[test]
    fn gauge_set_overwrites() {
        let registry = MetricsRegistry::new();
        let g = registry.gauge("cpu_usage", "CPU usage");
        g.set(50.0, &[("core", "0")]);
        g.set(75.5, &[("core", "0")]);

        let snapshots = registry.collect();
        assert_eq!(snapshots.len(), 1);
        match &snapshots[0] {
            MetricSnapshot::Gauge {
                name, data_points, ..
            } => {
                assert_eq!(name, "cpu_usage");
                assert_eq!(data_points.len(), 1);
                assert!((data_points[0].1 - 75.5).abs() < f64::EPSILON);
            }
            _ => panic!("expected Gauge snapshot"),
        }
    }

    #[test]
    fn counter_no_attrs() {
        let registry = MetricsRegistry::new();
        let c = registry.counter("simple", "simple counter");
        c.add(10, &[]);

        let snapshots = registry.collect();
        assert_eq!(snapshots.len(), 1);
        match &snapshots[0] {
            MetricSnapshot::Counter { data_points, .. } => {
                assert_eq!(data_points.len(), 1);
                assert_eq!(data_points[0].1, 10);
                assert!(data_points[0].0.is_empty());
            }
            _ => panic!("expected Counter"),
        }
    }

    #[test]
    fn empty_registry_collects_nothing() {
        let registry = MetricsRegistry::new();
        let _ = registry.counter("unused", "never incremented");
        assert!(registry.collect().is_empty());
    }

    #[test]
    fn counter_clone_shares_state() {
        let registry = MetricsRegistry::new();
        let c1 = registry.counter("shared", "shared counter");
        let c2 = c1.clone();
        c1.add(5, &[]);
        c2.add(3, &[]);

        let snapshots = registry.collect();
        match &snapshots[0] {
            MetricSnapshot::Counter { data_points, .. } => {
                assert_eq!(data_points[0].1, 8);
            }
            _ => panic!("expected Counter"),
        }
    }

    #[test]
    fn attrs_order_does_not_matter() {
        let registry = MetricsRegistry::new();
        let c = registry.counter("order_test", "test");
        c.add(1, &[("a", "1"), ("b", "2")]);
        c.add(1, &[("b", "2"), ("a", "1")]);

        let snapshots = registry.collect();
        match &snapshots[0] {
            MetricSnapshot::Counter { data_points, .. } => {
                assert_eq!(data_points.len(), 1);
                assert_eq!(data_points[0].1, 2);
            }
            _ => panic!("expected Counter"),
        }
    }

    #[test]
    fn histogram_observe_accumulates() {
        let registry = MetricsRegistry::new();
        let h = registry.histogram("latency", "request latency", &[10.0, 50.0, 100.0]);
        h.observe(5.0, &[("method", "GET")]);
        h.observe(25.0, &[("method", "GET")]);
        h.observe(75.0, &[("method", "GET")]);
        h.observe(200.0, &[("method", "GET")]);

        let snapshots = registry.collect();
        assert_eq!(snapshots.len(), 1);
        match &snapshots[0] {
            MetricSnapshot::Histogram {
                name,
                boundaries,
                data_points,
                ..
            } => {
                assert_eq!(name, "latency");
                assert_eq!(boundaries, &[10.0, 50.0, 100.0]);
                assert_eq!(data_points.len(), 1);
                let dp = &data_points[0];
                // 4 buckets: [0,10), [10,50), [50,100), [100,+inf)
                assert_eq!(dp.bucket_counts, vec![1, 1, 1, 1]);
                assert_eq!(dp.count, 4);
                assert!((dp.sum - 305.0).abs() < f64::EPSILON);
                assert!((dp.min - 5.0).abs() < f64::EPSILON);
                assert!((dp.max - 200.0).abs() < f64::EPSILON);
            }
            _ => panic!("expected Histogram snapshot"),
        }
    }

    #[test]
    fn histogram_boundary_placement() {
        let registry = MetricsRegistry::new();
        let h = registry.histogram("bp", "test", &[10.0, 20.0]);
        // Exactly on boundary goes to the next bucket
        h.observe(10.0, &[]);
        h.observe(20.0, &[]);
        h.observe(0.0, &[]);

        let snapshots = registry.collect();
        match &snapshots[0] {
            MetricSnapshot::Histogram { data_points, .. } => {
                let dp = &data_points[0];
                // [0,10) = 1 (0.0), [10,20) = 1 (10.0), [20,+inf) = 1 (20.0)
                assert_eq!(dp.bucket_counts, vec![1, 1, 1]);
            }
            _ => panic!("expected Histogram"),
        }
    }

    #[test]
    fn histogram_multiple_attr_sets() {
        let registry = MetricsRegistry::new();
        let h = registry.histogram("multi", "test", &[50.0]);
        h.observe(10.0, &[("method", "GET")]);
        h.observe(60.0, &[("method", "POST")]);

        let snapshots = registry.collect();
        match &snapshots[0] {
            MetricSnapshot::Histogram { data_points, .. } => {
                assert_eq!(data_points.len(), 2);
            }
            _ => panic!("expected Histogram"),
        }
    }

    #[test]
    fn histogram_clone_shares_state() {
        let registry = MetricsRegistry::new();
        let h1 = registry.histogram("shared_h", "test", &[10.0]);
        let h2 = h1.clone();
        h1.observe(5.0, &[]);
        h2.observe(15.0, &[]);

        let snapshots = registry.collect();
        match &snapshots[0] {
            MetricSnapshot::Histogram { data_points, .. } => {
                let dp = &data_points[0];
                assert_eq!(dp.count, 2);
            }
            _ => panic!("expected Histogram"),
        }
    }

    #[test]
    fn histogram_empty_not_collected() {
        let registry = MetricsRegistry::new();
        let _ = registry.histogram("unused_h", "test", &[10.0]);
        assert!(registry.collect().is_empty());
    }

    #[test]
    fn histogram_no_attrs() {
        let registry = MetricsRegistry::new();
        let h = registry.histogram("no_attrs_h", "test", &[5.0]);
        h.observe(1.0, &[]);

        let snapshots = registry.collect();
        match &snapshots[0] {
            MetricSnapshot::Histogram { data_points, .. } => {
                assert_eq!(data_points.len(), 1);
                assert!(data_points[0].attrs.is_empty());
            }
            _ => panic!("expected Histogram"),
        }
    }

    #[test]
    fn counter_no_exemplar_without_span() {
        // Without an active tracing span, exemplar should be None.
        let registry = MetricsRegistry::new();
        let c = registry.counter("no_span", "test");
        c.add(1, &[]);

        let snapshots = registry.collect();
        match &snapshots[0] {
            MetricSnapshot::Counter { data_points, .. } => {
                assert!(data_points[0].2.is_none());
            }
            _ => panic!("expected Counter"),
        }
    }

    #[test]
    fn exemplar_resets_on_collect() {
        // Manually insert an exemplar and verify it's consumed on collect.
        let registry = MetricsRegistry::new();
        let c = registry.counter("reset_test", "test");
        c.add(1, &[]);

        // Inject a fake exemplar directly.
        {
            let counters = registry.counters.read().unwrap();
            let counter = counters.get("reset_test").unwrap();
            let mut data = counter.inner.data.lock().unwrap();
            for entry in data.values_mut() {
                entry.2 = Some(Exemplar {
                    trace_id: [0xAA; 16],
                    span_id: [0xBB; 8],
                    time_unix_nano: 123_456,
                    value: ExemplarValue::Int(1),
                });
            }
        }

        // First collect should yield the exemplar.
        let snap1 = registry.collect();
        match &snap1[0] {
            MetricSnapshot::Counter { data_points, .. } => {
                assert!(data_points[0].2.is_some());
            }
            _ => panic!("expected Counter"),
        }

        // Second collect should have None (reset by .take()).
        let snap2 = registry.collect();
        match &snap2[0] {
            MetricSnapshot::Counter { data_points, .. } => {
                assert!(data_points[0].2.is_none());
            }
            _ => panic!("expected Counter"),
        }
    }
}
