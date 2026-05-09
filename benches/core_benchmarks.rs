use chrono::Utc;
use criterion::{Criterion, criterion_group, criterion_main};
use rafn::proto::{Benchmark, Metrics};
use std::hint::black_box;
use uuid::Uuid;

fn benchmark_builder_benchmark(c: &mut Criterion) {
    let tenant_id = Uuid::new_v4();
    let metrics = Metrics::new(1000.0, 900.0, 100.0, 800.0, 1200.0);

    c.bench_function("benchmark_builder_build", |b| {
        b.iter(|| {
            Benchmark::builder()
                .tenant_id(black_box(tenant_id))
                .repository(black_box("test-repo".to_string()))
                .commit_sha(black_box("abc123def456".to_string()))
                .benchmark_name(black_box("test_benchmark".to_string()))
                .toolset(black_box("criterion".to_string()))
                .language(black_box("rust".to_string()))
                .metrics(black_box(metrics.clone()))
                .timestamp(black_box(Utc::now()))
                .build()
                .unwrap()
        })
    });
}

fn metrics_creation_benchmark(c: &mut Criterion) {
    c.bench_function("metrics_new", |b| {
        b.iter(|| {
            Metrics::new(
                black_box(1000.0),
                black_box(900.0),
                black_box(100.0),
                black_box(800.0),
                black_box(1200.0),
            )
        })
    });

    c.bench_function("metrics_from_seconds", |b| {
        b.iter(|| Metrics::from_seconds(black_box(1.0)))
    });

    c.bench_function("metrics_from_milliseconds", |b| {
        b.iter(|| Metrics::from_milliseconds(black_box(1.0)))
    });
}

criterion_group!(
    benches,
    benchmark_builder_benchmark,
    metrics_creation_benchmark
);
criterion_main!(benches);
