use criterion::{Criterion, criterion_group, criterion_main};
use rafn::proto::benchmark::{
    benchmark_record, benchmark_set, metric_statistics, milliseconds_to_ns, seconds_to_ns,
};
use std::hint::black_box;

fn benchmark_set_benchmark(c: &mut Criterion) {
    let statistics = metric_statistics(1000.0, 900.0, 100.0, 800.0, 1200.0, Some(50));

    c.bench_function("benchmark_set_build", |b| {
        b.iter(|| {
            let benchmark = benchmark_record(
                black_box("test_benchmark".to_string()),
                black_box(statistics),
            );
            benchmark_set(
                black_box("owner/test-repo"),
                black_box("abc123def456"),
                None,
                black_box("run-1".to_string()),
                black_box(prost_types::Timestamp::default()),
                black_box("rust"),
                black_box("criterion"),
                vec![benchmark],
            )
        })
    });
}

fn metric_statistics_benchmark(c: &mut Criterion) {
    c.bench_function("metric_statistics_new", |b| {
        b.iter(|| {
            metric_statistics(
                black_box(1000.0),
                black_box(900.0),
                black_box(100.0),
                black_box(800.0),
                black_box(1200.0),
                black_box(Some(50)),
            )
        })
    });

    c.bench_function("metrics_from_seconds", |b| {
        b.iter(|| seconds_to_ns(black_box(1.0)))
    });

    c.bench_function("metrics_from_milliseconds", |b| {
        b.iter(|| milliseconds_to_ns(black_box(1.0)))
    });
}

criterion_group!(
    benches,
    benchmark_set_benchmark,
    metric_statistics_benchmark
);
criterion_main!(benches);
