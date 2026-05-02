#![allow(unused_crate_dependencies)]
use criterion::{criterion_group, criterion_main, Criterion};
use ingest::parsers::criterion::CriterionParser;
use ingest::BenchmarkParser;
use std::hint::black_box;

fn criterion_parser_benchmark(c: &mut Criterion) {
    // Sample Criterion JSON output for testing
    let sample_json = r#"{
        "reason": "benchmark-complete",
        "id": "sample_benchmark",
        "report_directory": "/tmp/criterion/sample",
        "iteration_count": [1000, 2000, 3000],
        "measured_values": [500000, 1000000, 1500000],
        "unit": "ns",
        "throughput": [],
        "typical": {
            "point_estimate": 500000.0,
            "lower_bound": 450000.0,
            "upper_bound": 550000.0,
            "unit": "ns"
        },
        "mean": {
            "point_estimate": 500000.0,
            "lower_bound": 450000.0,
            "upper_bound": 550000.0,
            "unit": "ns",
            "confidence_interval": {
                "confidence_level": 0.95,
                "lower_bound": 450000.0,
                "upper_bound": 550000.0
            }
        },
        "median": {
            "point_estimate": 495000.0,
            "lower_bound": 490000.0,
            "upper_bound": 500000.0,
            "unit": "ns",
            "confidence_interval": {
                "confidence_level": 0.95,
                "lower_bound": 490000.0,
                "upper_bound": 500000.0
            }
        },
        "std_dev": {
            "point_estimate": 25000.0,
            "lower_bound": 20000.0,
            "upper_bound": 30000.0,
            "unit": "ns",
            "confidence_interval": {
                "confidence_level": 0.95,
                "lower_bound": 20000.0,
                "upper_bound": 30000.0
            }
        }
    }"#;

    let parser = CriterionParser::new(
        uuid::Uuid::new_v4(),
        "test-repo".to_string(),
        "abc123".to_string(),
    );

    c.bench_function("parse_criterion_json", |b| {
        b.iter(|| parser.parse(black_box(sample_json)).unwrap())
    });
}

criterion_group!(benches, criterion_parser_benchmark);
criterion_main!(benches);
