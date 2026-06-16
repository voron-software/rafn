//! Shared benchmark comparison logic used by `bench` and `compare` commands.

use std::collections::HashMap;

use colored::Colorize;
use serde::Serialize;
use tabled::{Table, Tabled};

use crate::proto::benchmark::statistic_mean_ns;
use crate::proto::pb::BenchmarkSet;

pub fn format_duration(ns: &f64) -> String {
    let v = *ns;
    if v.abs() < 1_000.0 {
        format!("{v:.3} ns")
    } else if v.abs() < 1_000_000.0 {
        format!("{:.3} µs", v / 1_000.0)
    } else if v.abs() < 1_000_000_000.0 {
        format!("{:.3} ms", v / 1_000_000.0)
    } else {
        format!("{:.3} s", v / 1_000_000_000.0)
    }
}

pub fn format_diff(ns: &f64) -> String {
    let v = *ns;
    let sign = if v > 0.0 { "+" } else { "" };
    if v.abs() < 1_000.0 {
        format!("{sign}{v:.3} ns")
    } else if v.abs() < 1_000_000.0 {
        format!("{sign}{:.3} µs", v / 1_000.0)
    } else if v.abs() < 1_000_000_000.0 {
        format!("{sign}{:.3} ms", v / 1_000_000.0)
    } else {
        format!("{sign}{:.3} s", v / 1_000_000_000.0)
    }
}

pub fn format_percent(pct: &f64) -> String {
    let s = if *pct > 0.0 {
        format!("+{pct:.1}%")
    } else {
        format!("{pct:.1}%")
    };
    if *pct > 0.0 {
        s.red().to_string()
    } else if *pct < 0.0 {
        s.green().to_string()
    } else {
        s
    }
}

#[derive(Debug, Clone, Serialize, Tabled)]
pub struct ComparisonRow {
    #[tabled(rename = "Benchmark")]
    pub benchmark_name: String,
    #[tabled(rename = "Base", display = "format_duration")]
    pub base_mean_ns: f64,
    #[tabled(rename = "Head", display = "format_duration")]
    pub head_mean_ns: f64,
    #[tabled(rename = "Diff", display = "format_diff")]
    pub diff_ns: f64,
    #[tabled(rename = "Change %", display = "format_percent")]
    pub diff_pct: f64,
}

/// Compute per-benchmark diffs between `base` and `head` snapshots.
/// Only benchmarks present in both snapshots are included.
/// Results are sorted by absolute diff (largest first).
pub fn compare(base: &[BenchmarkSet], head: &[BenchmarkSet]) -> Vec<ComparisonRow> {
    let base_map = flatten_means(base);
    let head_map = flatten_means(head);

    let mut rows: Vec<ComparisonRow> = base_map
        .iter()
        .filter_map(|(benchmark_name, base_mean_ns)| {
            let head_mean_ns = head_map.get(benchmark_name)?;
            let diff_ns = *head_mean_ns - *base_mean_ns;
            let diff_pct = if *base_mean_ns > 0.0 {
                (diff_ns / *base_mean_ns) * 100.0
            } else {
                0.0
            };
            Some(ComparisonRow {
                benchmark_name: benchmark_name.clone(),
                base_mean_ns: *base_mean_ns,
                head_mean_ns: *head_mean_ns,
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

fn flatten_means(sets: &[BenchmarkSet]) -> HashMap<String, f64> {
    sets.iter()
        .filter(|set| set.metric_name == "wall_time")
        .flat_map(|set| set.benchmarks.iter())
        .filter_map(|benchmark| {
            statistic_mean_ns(benchmark).map(|mean| (benchmark.name.clone(), mean))
        })
        .collect()
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
    use crate::config::RepositoryRef;
    use crate::proto::benchmark::{benchmark_record, benchmark_set, metric_statistics};

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn format_duration_selects_ns_below_1000() {
        assert_eq!(format_duration(&0.0), "0.000 ns");
        assert_eq!(format_duration(&999.9), "999.900 ns");
    }

    #[test]
    fn format_duration_selects_us_below_1_000_000() {
        assert_eq!(format_duration(&1_000.0), "1.000 µs");
        assert_eq!(format_duration(&2_338.271), "2.338 µs");
    }

    #[test]
    fn format_duration_selects_ms_below_1_000_000_000() {
        assert_eq!(format_duration(&1_000_000.0), "1.000 ms");
        assert_eq!(format_duration(&17_356_000.0), "17.356 ms");
    }

    #[test]
    fn format_duration_selects_s_above_1_000_000_000() {
        assert_eq!(format_duration(&1_000_000_000.0), "1.000 s");
        assert_eq!(format_duration(&2_500_000_000.0), "2.500 s");
    }

    #[test]
    fn format_diff_prefixes_positive_with_plus() {
        assert_eq!(format_diff(&17.720), "+17.720 ns");
        assert_eq!(format_diff(&1_500_000.0), "+1.500 ms");
    }

    #[test]
    fn format_diff_no_plus_for_negative_or_zero() {
        assert_eq!(format_diff(&-1_149.582), "-1.150 µs");
        assert_eq!(format_diff(&0.0), "0.000 ns");
    }

    #[test]
    fn format_percent_positive_is_red_with_plus() {
        colored::control::set_override(true);
        let out = format_percent(&10.0);
        colored::control::unset_override();
        assert_eq!(strip_ansi(&out), "+10.0%");
        assert!(
            out.contains("\x1b["),
            "expected ANSI color codes for regression"
        );
    }

    #[test]
    fn format_percent_negative_is_green_without_plus() {
        colored::control::set_override(true);
        let out = format_percent(&-7.5);
        colored::control::unset_override();
        assert_eq!(strip_ansi(&out), "-7.5%");
        assert!(
            out.contains("\x1b["),
            "expected ANSI color codes for improvement"
        );
    }

    #[test]
    fn format_percent_zero_is_plain() {
        let out = format_percent(&0.0);
        assert_eq!(out, "0.0%");
        assert!(
            !out.contains("\x1b["),
            "zero change should have no ANSI codes"
        );
    }

    fn test_repository() -> RepositoryRef {
        RepositoryRef {
            forge: "github.com".to_string(),
            owner: "owner".to_string(),
            repository: "repo".to_string(),
        }
    }

    fn make_set(name: &str, mean_ns: f64) -> BenchmarkSet {
        benchmark_set(
            &test_repository(),
            "abc123",
            None,
            "run-1".to_string(),
            prost_types::Timestamp::default(),
            "rust",
            "criterion",
            vec![benchmark_record(
                name.to_string(),
                metric_statistics(mean_ns, 0.0, 0.0, 0.0, 0.0, None),
            )],
        )
    }

    #[test]
    fn test_compare_computes_diff() {
        let base = vec![make_set("foo", 1_000_000.0)];
        let head = vec![make_set("foo", 1_100_000.0)];
        let rows = compare(&base, &head);
        assert_eq!(rows.len(), 1);
        assert!((rows[0].diff_pct - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_compare_skips_unmatched() {
        let base = vec![make_set("foo", 1_000_000.0), make_set("bar", 2_000_000.0)];
        let head = vec![make_set("foo", 1_000_000.0)];
        let rows = compare(&base, &head);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].benchmark_name, "foo");
    }

    #[test]
    fn test_has_regressions_above_threshold() {
        let base = vec![make_set("foo", 1_000_000.0)];
        let head = vec![make_set("foo", 1_100_000.0)]; // +10%
        let rows = compare(&base, &head);
        assert!(has_regressions(&rows, 5.0));
        assert!(!has_regressions(&rows, 15.0));
    }

    #[test]
    fn test_has_regressions_improvement_does_not_trip() {
        let base = vec![make_set("foo", 1_100_000.0)];
        let head = vec![make_set("foo", 1_000_000.0)]; // improvement
        let rows = compare(&base, &head);
        assert!(!has_regressions(&rows, 5.0));
    }
}
