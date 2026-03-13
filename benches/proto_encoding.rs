use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rolly::bench::*;

fn bench_encode_message_field_in_place(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_message_field_in_place");

    let small_body = vec![0x42u8; 10];
    let medium_body = vec![0x42u8; 200];
    let large_body = vec![0x42u8; 10_000];

    group.bench_function("small_10B", |b| {
        let body = small_body.clone();
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            encode_message_field_in_place(&mut buf, 1, |buf| {
                buf.extend_from_slice(black_box(&body));
            });
            black_box(&buf);
        });
    });

    group.bench_function("medium_200B", |b| {
        let body = medium_body.clone();
        b.iter(|| {
            let mut buf = Vec::with_capacity(256);
            encode_message_field_in_place(&mut buf, 1, |buf| {
                buf.extend_from_slice(black_box(&body));
            });
            black_box(&buf);
        });
    });

    group.bench_function("large_10KB", |b| {
        let body = large_body.clone();
        b.iter(|| {
            let mut buf = Vec::with_capacity(10_100);
            encode_message_field_in_place(&mut buf, 1, |buf| {
                buf.extend_from_slice(black_box(&body));
            });
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_key_value(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_key_value");

    let string_kv = KeyValue {
        key: "http.method".to_string(),
        value: AnyValue::String("GET".to_string()),
    };
    let int_kv = KeyValue {
        key: "http.status_code".to_string(),
        value: AnyValue::Int(200),
    };
    let bool_kv = KeyValue {
        key: "http.retry".to_string(),
        value: AnyValue::Bool(true),
    };

    group.bench_function("string", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            encode_key_value(&mut buf, black_box(&string_kv));
            black_box(&buf);
        });
    });

    group.bench_function("int", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            encode_key_value(&mut buf, black_box(&int_kv));
            black_box(&buf);
        });
    });

    group.bench_function("bool", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            encode_key_value(&mut buf, black_box(&bool_kv));
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_resource(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_resource");

    let make_attrs = |n: usize| -> Vec<KeyValue> {
        (0..n)
            .map(|i| KeyValue {
                key: format!("attr.key.{}", i),
                value: AnyValue::String(format!("value-{}", i)),
            })
            .collect()
    };

    let attrs_1 = make_attrs(1);
    let attrs_5 = make_attrs(5);
    let attrs_20 = make_attrs(20);

    group.bench_function("1_attr", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            encode_resource(&mut buf, black_box(&attrs_1));
            black_box(&buf);
        });
    });

    group.bench_function("5_attrs", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(256);
            encode_resource(&mut buf, black_box(&attrs_5));
            black_box(&buf);
        });
    });

    group.bench_function("20_attrs", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(1024);
            encode_resource(&mut buf, black_box(&attrs_20));
            black_box(&buf);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_encode_message_field_in_place,
    bench_encode_key_value,
    bench_encode_resource,
);
criterion_main!(benches);
