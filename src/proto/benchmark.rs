use std::time::{SystemTime, UNIX_EPOCH};

use super::pb;

pub fn toolset_enum(s: &str) -> pb::Toolset {
    match s {
        "criterion" => pb::Toolset::Criterion,
        "divan" => pb::Toolset::Divan,
        "jmh" => pb::Toolset::Jmh,
        "google_benchmark" => pb::Toolset::GoogleBenchmark,
        "benchmarkdotnet" => pb::Toolset::BenchmarkDotnet,
        "go_test" => pb::Toolset::GoTest,
        "pytest_benchmark" => pb::Toolset::PytestBenchmark,
        "pyperf" => pb::Toolset::Pyperf,
        "vitest_bench" => pb::Toolset::VitestBench,
        "benchmark_js" => pb::Toolset::BenchmarkJs,
        "catch2" => pb::Toolset::Catch2,
        _ => pb::Toolset::Unspecified,
    }
}

pub fn language_enum(s: &str) -> pb::Language {
    match s {
        "rust" => pb::Language::Rust,
        "go" => pb::Language::Go,
        "java" => pb::Language::Java,
        "kotlin" => pb::Language::Kotlin,
        "csharp" => pb::Language::Csharp,
        "fsharp" => pb::Language::Fsharp,
        "cpp" => pb::Language::Cpp,
        "c" => pb::Language::C,
        "python" => pb::Language::Python,
        "javascript" => pb::Language::Javascript,
        "typescript" => pb::Language::Typescript,
        _ => pb::Language::Unspecified,
    }
}

pub fn split_repository(repository: &str) -> (String, String) {
    match repository.split_once('/') {
        Some((owner, repo)) => (owner.to_string(), repo.to_string()),
        None => (String::new(), repository.to_string()),
    }
}

pub fn timestamp_now() -> prost_types::Timestamp {
    timestamp_from_system_time(SystemTime::now())
}

pub fn timestamp_from_system_time(time: SystemTime) -> prost_types::Timestamp {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    prost_types::Timestamp {
        seconds: duration.as_secs() as i64,
        nanos: duration.subsec_nanos() as i32,
    }
}

pub fn timestamp_to_millis(ts: &prost_types::Timestamp) -> i64 {
    ts.seconds * 1000 + i64::from(ts.nanos) / 1_000_000
}

pub fn seconds_to_ns(seconds: f64) -> f64 {
    seconds * 1_000_000_000.0
}

pub fn milliseconds_to_ns(ms: f64) -> f64 {
    ms * 1_000_000.0
}

pub fn microseconds_to_ns(us: f64) -> f64 {
    us * 1_000.0
}

pub fn metric_statistics(
    mean: f64,
    median: f64,
    stddev: f64,
    min: f64,
    max: f64,
    sample_count: Option<u64>,
) -> pb::MetricStatistics {
    pb::MetricStatistics {
        mean: Some(mean),
        median: Some(median),
        stddev: Some(stddev),
        min: Some(min),
        max: Some(max),
        sample_count,
        p50: None,
        p90: None,
        p95: None,
        p99: None,
    }
}

pub fn benchmark_record(name: String, statistics: pb::MetricStatistics) -> pb::Benchmark {
    pb::Benchmark {
        name,
        location: None,
        parameters: Default::default(),
        samples: Vec::new(),
        statistics: Some(statistics),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn benchmark_set(
    repository: &str,
    commit_sha: &str,
    branch: Option<String>,
    run_uuid: String,
    run_started_at: prost_types::Timestamp,
    language: &str,
    toolset: &str,
    benchmarks: Vec<pb::Benchmark>,
) -> pb::BenchmarkSet {
    let (owner, repo) = split_repository(repository);
    pb::BenchmarkSet {
        run_uuid,
        source: Some(pb::SourceInformation {
            forge: "github.com".to_string(),
            owner,
            repository: repo,
            commit_sha: commit_sha.to_string(),
            commit_graph: None,
            branch,
            tag: None,
            dirty: false,
        }),
        toolset: Some(pb::ToolsetInformation {
            language: language_enum(language) as i32,
            language_other: None,
            language_version: None,
            toolset: toolset_enum(toolset) as i32,
            toolset_other: None,
            toolset_version: None,
        }),
        machine: None,
        ci: None,
        metric_name: "wall_time".to_string(),
        unit: pb::Unit::Nanoseconds as i32,
        benchmarks,
        labels: Default::default(),
        run_started_at: Some(run_started_at),
    }
}

pub fn statistic_mean_ns(benchmark: &pb::Benchmark) -> Option<f64> {
    benchmark.statistics.as_ref().and_then(|s| s.mean)
}

pub fn statistic_median_ns(benchmark: &pb::Benchmark) -> f64 {
    benchmark
        .statistics
        .as_ref()
        .and_then(|s| s.median)
        .unwrap_or(0.0)
}

pub fn statistic_stddev_ns(benchmark: &pb::Benchmark) -> f64 {
    benchmark
        .statistics
        .as_ref()
        .and_then(|s| s.stddev)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_repository_handles_no_slash() {
        let (owner, repo) = split_repository("standalone");
        assert_eq!(owner, "");
        assert_eq!(repo, "standalone");
    }

    #[test]
    fn split_repository_splits_owner_repo() {
        let (owner, repo) = split_repository("acme/perf-suite");
        assert_eq!(owner, "acme");
        assert_eq!(repo, "perf-suite");
    }

    #[test]
    fn timestamp_to_millis_converts_correctly() {
        let ts = prost_types::Timestamp {
            seconds: 1_700_000_000,
            nanos: 500_000_000,
        };
        assert_eq!(timestamp_to_millis(&ts), 1_700_000_000 * 1000 + 500);
    }
}
