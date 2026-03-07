use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// Global metrics registry.
static GLOBAL_REGISTRY: OnceLock<MetricsRegistry> = OnceLock::new();

/// Get or initialize the global registry.
pub fn global_registry() -> &'static MetricsRegistry {
    GLOBAL_REGISTRY.get_or_init(MetricsRegistry::new)
}

/// A snapshot of a single metric for encoding.
pub enum MetricSnapshot {
    Counter {
        name: String,
        description: String,
        /// Each entry: (sorted attribute pairs, cumulative value)
        data_points: Vec<(Vec<(String, String)>, i64)>,
    },
    Gauge {
        name: String,
        description: String,
        /// Each entry: (sorted attribute pairs, last value)
        data_points: Vec<(Vec<(String, String)>, f64)>,
    },
}

/// Central registry holding all counters and gauges.
pub struct MetricsRegistry {
    counters: RwLock<HashMap<String, Counter>>,
    gauges: RwLock<HashMap<String, Gauge>>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            counters: RwLock::new(HashMap::new()),
            gauges: RwLock::new(HashMap::new()),
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

    /// Snapshot all metrics for encoding. Does not reset counters (cumulative).
    pub fn collect(&self) -> Vec<MetricSnapshot> {
        let mut snapshots = Vec::new();

        {
            let counters = self.counters.read().unwrap();
            for counter in counters.values() {
                let data = counter.inner.data.lock().unwrap();
                if data.is_empty() {
                    continue;
                }
                let data_points: Vec<_> = data.values().cloned().collect();
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
                let data = gauge.inner.data.lock().unwrap();
                if data.is_empty() {
                    continue;
                }
                let data_points: Vec<_> = data.values().cloned().collect();
                snapshots.push(MetricSnapshot::Gauge {
                    name: gauge.inner.name.clone(),
                    description: gauge.inner.description.clone(),
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

/// Sort and own attribute pairs.
fn owned_attrs(attrs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut owned: Vec<(String, String)> = attrs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    owned.sort();
    owned
}

// --- Counter ---

struct CounterInner {
    name: String,
    description: String,
    /// Key: hash of sorted attribute pairs. Value: (owned attrs, cumulative value).
    data: Mutex<HashMap<u64, (Vec<(String, String)>, i64)>>,
}

/// A monotonic u64 counter. Clone is cheap (Arc).
#[derive(Clone)]
pub struct Counter {
    inner: Arc<CounterInner>,
}

impl Counter {
    /// Add a value to the counter for the given attribute set.
    pub fn add(&self, value: u64, attrs: &[(&str, &str)]) {
        let mut sorted = attrs.to_vec();
        sorted.sort();
        let key = attrs_hash(&sorted);
        let mut data = self.inner.data.lock().unwrap();
        let entry = data
            .entry(key)
            .or_insert_with(|| (owned_attrs(&sorted), 0));
        entry.1 += value as i64;
    }
}

// --- Gauge ---

struct GaugeInner {
    name: String,
    description: String,
    /// Key: hash of sorted attribute pairs. Value: (owned attrs, last value).
    data: Mutex<HashMap<u64, (Vec<(String, String)>, f64)>>,
}

/// A last-value f64 gauge. Clone is cheap (Arc).
#[derive(Clone)]
pub struct Gauge {
    inner: Arc<GaugeInner>,
}

impl Gauge {
    /// Set the gauge to a value for the given attribute set.
    pub fn set(&self, value: f64, attrs: &[(&str, &str)]) {
        let mut sorted = attrs.to_vec();
        sorted.sort();
        let key = attrs_hash(&sorted);
        let mut data = self.inner.data.lock().unwrap();
        let entry = data
            .entry(key)
            .or_insert_with(|| (owned_attrs(&sorted), 0.0));
        entry.1 = value;
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
            MetricSnapshot::Counter { name, data_points, .. } => {
                assert_eq!(name, "req_total");
                assert_eq!(data_points.len(), 2);
                let get_val = data_points
                    .iter()
                    .find(|(a, _)| a[0].1 == "GET")
                    .unwrap()
                    .1;
                assert_eq!(get_val, 4);
                let post_val = data_points
                    .iter()
                    .find(|(a, _)| a[0].1 == "POST")
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
            MetricSnapshot::Gauge { name, data_points, .. } => {
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
}
