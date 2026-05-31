//! Shared benchmark comparison logic used by `bench` and `compare` commands.

use std::collections::HashMap;

use colored::Colorize;
use serde::Serialize;
use tabled::{Table, Tabled};

use crate::proto::Benchmark;

pub fn format_duration_ms(ns: &f64) -> String {
    format!("{:.3}", ns / 1_000_000.0)
}

pub fn format_diff_ms(ns: &f64) -> String {
    let ms = ns / 1_000_000.0;
    if *ns > 0.0 {
        format!("+{ms:.3}")
    } else {
        format!("{ms:.3}")
    }
}

pub fn format_percent(pct: &f64) -> String {
    if *pct > 0.0 {
        format!("+{pct:.1}%")
    } else {
        format!("{pct:.1}%")
    }
}

#[derive(Debug, Clone, Serialize, Tabled)]
pub struct ComparisonRow {
    #[tabled(rename = "Benchmark")]
    pub benchmark_name: String,
    #[tabled(rename = "Base (ms)", display = "format_duration_ms")]
    pub base_mean_ns: f64,
    #[tabled(rename = "Head (ms)", display = "format_duration_ms")]
    pub head_mean_ns: f64,
    #[tabled(rename = "Diff (ms)", display = "format_diff_ms")]
    pub diff_ns: f64,
    #[tabled(rename = "Change %", display = "format_percent")]
    pub diff_pct: f64,
}

/// Compute per-benchmark diffs between `base` and `head` snapshots.
/// Only benchmarks present in both snapshots are included.
/// Results are sorted by absolute diff (largest first).
pub fn compare(base: &[Benchmark], head: &[Benchmark]) -> Vec<ComparisonRow> {
    let head_map: HashMap<&str, &Benchmark> = head
        .iter()
        .map(|b| (b.benchmark_name.as_str(), b))
        .collect();

    let mut rows: Vec<ComparisonRow> = base
        .iter()
        .filter_map(|base_bench| {
            let head_bench = head_map.get(base_bench.benchmark_name.as_str())?;
            let diff_ns = head_bench.metrics.mean_ns - base_bench.metrics.mean_ns;
            let diff_pct = if base_bench.metrics.mean_ns > 0.0 {
                (diff_ns / base_bench.metrics.mean_ns) * 100.0
            } else {
                0.0
            };
            Some(ComparisonRow {
                benchmark_name: base_bench.benchmark_name.clone(),
                base_mean_ns: base_bench.metrics.mean_ns,
                head_mean_ns: head_bench.metrics.mean_ns,
                diff_ns,
                diff_pct,
            })
        })
        .collect();

    rows.sort_by(|a, b| {
        b.diff_ns
            .abs()
            .partial_cmp(&a.diff_ns.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

/// Return `true` if any benchmark in `rows` regressed by more than `threshold`
/// percent. A regression is defined as `diff_pct > threshold` (slower is positive).
pub fn has_regressions(rows: &[ComparisonRow], threshold: f64) -> bool {
    rows.iter().any(|r| r.diff_pct > threshold)
}

/// Print a comparison table and a summary line to stdout.
pub fn print_table(rows: &[ComparisonRow]) {
    let table = Table::new(rows).to_string();
    println!("{table}");
    println!();

    let improvements = rows.iter().filter(|r| r.diff_ns < 0.0).count();
    let regressions = rows.iter().filter(|r| r.diff_ns > 0.0).count();
    println!("Summary:");
    println!("  Total: {}", rows.len());
    println!("  {} Improvements: {}", "✓".green(), improvements);
    println!("  {} Regressions: {}", "✗".red(), regressions);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Benchmark, Metrics};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_bench(name: &str, mean_ns: f64) -> Benchmark {
        Benchmark {
            tenant_id: Uuid::nil(),
            repository: "r".into(),
            commit_sha: "c".into(),
            benchmark_name: name.into(),
            timestamp: Utc::now(),
            toolset: "criterion".into(),
            language: "rust".into(),
            branch: None,
            tag: None,
            ci_job_id: None,
            metrics: Metrics {
                mean_ns,
                ..Default::default()
            },
            custom_metrics: Default::default(),
            labels: Default::default(),
            cpu_model: None,
            os: None,
            raw_json: None,
        }
    }

    #[test]
    fn test_compare_computes_diff() {
        let base = vec![make_bench("foo", 1_000_000.0)];
        let head = vec![make_bench("foo", 1_100_000.0)];
        let rows = compare(&base, &head);
        assert_eq!(rows.len(), 1);
        assert!((rows[0].diff_pct - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_compare_skips_unmatched() {
        let base = vec![
            make_bench("foo", 1_000_000.0),
            make_bench("bar", 2_000_000.0),
        ];
        let head = vec![make_bench("foo", 1_000_000.0)];
        let rows = compare(&base, &head);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].benchmark_name, "foo");
    }

    #[test]
    fn test_has_regressions_above_threshold() {
        let base = vec![make_bench("foo", 1_000_000.0)];
        let head = vec![make_bench("foo", 1_100_000.0)]; // +10%
        let rows = compare(&base, &head);
        assert!(has_regressions(&rows, 5.0));
        assert!(!has_regressions(&rows, 15.0));
    }

    #[test]
    fn test_has_regressions_improvement_does_not_trip() {
        let base = vec![make_bench("foo", 1_100_000.0)];
        let head = vec![make_bench("foo", 1_000_000.0)]; // improvement
        let rows = compare(&base, &head);
        assert!(!has_regressions(&rows, 5.0));
    }
}
