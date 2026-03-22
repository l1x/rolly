use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use rolly::bench::*;
use tracing_subscriber::layer::SubscriberExt;

/// Create a Dispatch backed by an OtlpLayer with a large-capacity test exporter.
/// Returns the dispatch and receiver (hold it to keep the channel open).
fn make_dispatch(
    capacity: usize,
    sampling_rate: f64,
) -> (
    tracing::Dispatch,
    tokio::sync::mpsc::Receiver<ExportMessage>,
) {
    let (exporter, rx) = Exporter::start_test_with_capacity(capacity, BackpressureStrategy::Drop);
    let layer = OtlpLayer::new(OtlpLayerConfig {
        exporter,
        service_name: "bench-svc",
        service_version: "0.0.1",
        environment: "bench",
        resource_attributes: &[],
        export_traces: true,
        export_logs: true,
        sampling_rate,
    });
    let subscriber = tracing_subscriber::registry().with(layer);
    (tracing::Dispatch::new(subscriber), rx)
}

fn hot_path_span_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_path_span_lifecycle");

    group.bench_function("bare_span", |b| {
        let (dispatch, _rx) = make_dispatch(1_000_000, 1.0);
        let _guard = tracing::dispatcher::set_default(&dispatch);
        b.iter(|| {
            let span = tracing::info_span!("bench-span");
            let _enter = span.enter();
        });
    });

    group.bench_function("span_with_trace_id", |b| {
        let (dispatch, _rx) = make_dispatch(1_000_000, 1.0);
        let _guard = tracing::dispatcher::set_default(&dispatch);
        b.iter(|| {
            let span =
                tracing::info_span!("bench-span", trace_id = "aabbccdd11223344aabbccdd11223344");
            let _enter = span.enter();
        });
    });

    group.bench_function("span_5_attrs", |b| {
        let (dispatch, _rx) = make_dispatch(1_000_000, 1.0);
        let _guard = tracing::dispatcher::set_default(&dispatch);
        b.iter(|| {
            let span = tracing::info_span!(
                "bench-span",
                attr1 = "value1",
                attr2 = "value2",
                attr3 = 42i64,
                attr4 = true,
                attr5 = 2.72f64,
            );
            let _enter = span.enter();
        });
    });

    group.bench_function("span_with_event", |b| {
        let (dispatch, _rx) = make_dispatch(1_000_000, 1.0);
        let _guard = tracing::dispatcher::set_default(&dispatch);
        b.iter(|| {
            let span = tracing::info_span!("bench-span");
            let _enter = span.enter();
            tracing::info!("bench-event");
        });
    });

    group.finish();
}

fn hot_path_sampling(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_path_sampling");

    group.bench_function("should_sample_raw", |b| {
        let trace_id = [
            0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        b.iter(|| {
            black_box(should_sample(black_box(trace_id), black_box(0.5)));
        });
    });

    group.bench_function("span_sampled_out", |b| {
        let (dispatch, _rx) = make_dispatch(1_000_000, 0.0);
        let _guard = tracing::dispatcher::set_default(&dispatch);
        b.iter(|| {
            let span = tracing::info_span!("sampled-out");
            let _enter = span.enter();
        });
    });

    group.bench_function("span_sampled_in", |b| {
        let (dispatch, _rx) = make_dispatch(1_000_000, 1.0);
        let _guard = tracing::dispatcher::set_default(&dispatch);
        b.iter(|| {
            let span = tracing::info_span!("sampled-in");
            let _enter = span.enter();
        });
    });

    group.finish();
}

fn hot_path_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_path_throughput");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("1000_spans", |b| {
        let (dispatch, _rx) = make_dispatch(1_000_000, 1.0);
        let _guard = tracing::dispatcher::set_default(&dispatch);
        b.iter(|| {
            for _ in 0..1000 {
                let span = tracing::info_span!("throughput-span");
                let _enter = span.enter();
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    hot_path_span_lifecycle,
    hot_path_sampling,
    hot_path_throughput,
);
criterion_main!(benches);
