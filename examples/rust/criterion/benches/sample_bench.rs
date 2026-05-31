use criterion::{Criterion, black_box, criterion_group, criterion_main};
use criterion_example::{fibonacci, fibonacci_iterative};
use std::time::Duration;

fn bench_fibonacci(c: &mut Criterion) {
    let mut group = c.benchmark_group("fibonacci");
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_millis(500));
    group.sample_size(30);

    group.bench_function("recursive_20", |b| b.iter(|| fibonacci(black_box(20))));
    group.bench_function("iterative_20", |b| {
        b.iter(|| fibonacci_iterative(black_box(20)))
    });

    group.finish();
}

fn bench_fibonacci_group(c: &mut Criterion) {
    let mut group = c.benchmark_group("fibonacci_comparison");
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_millis(500));
    group.sample_size(30);

    for n in [10, 20].iter() {
        group.bench_function(format!("recursive_{}", n), |b| {
            b.iter(|| fibonacci(black_box(*n)))
        });
        group.bench_function(format!("iterative_{}", n), |b| {
            b.iter(|| fibonacci_iterative(black_box(*n)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_fibonacci, bench_fibonacci_group);
criterion_main!(benches);
