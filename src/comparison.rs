//! Shared benchmark comparison logic used by `bench` and `compare` commands.
//!
//! The types and `compare` algorithm here are a deliberate port of
//! `rafn-backend`'s `regression` crate (crates/regression), kept in sync by
//! hand rather than as a cross-repo dependency (rafn is a standalone binary
//! distributed via GitHub releases/winget, and pinning it to a git dependency
//! on rafn-backend would tie CLI releases to that repo's history). Keeping
//! the shape and field names identical means a local snapshot comparison and
//! a remote `CompareCommits` response ([`crate::store::remote`]) print and
//! serialize identically.

use std::collections::HashMap;

use anyhow::{Result, ensure};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use crate::proto::benchmark::statistic_mean_ns;
use crate::proto::pb::{Benchmark, BenchmarkSet, Unit, parameter_value};

/// Rejects a `--threshold`/`[bench].threshold` that cannot produce a
/// meaningful verdict: negative (an exact 0% change would already exceed
/// it, so it would classify as `Regressed`) or non-finite (`NaN` compares
/// false to everything, so `classify`'s `pct.abs() <= threshold_pct` would
/// never hold and every finite change would bypass `Unchanged`). Call this
/// before invoking either backend - local or remote - so `compare`/`bench`
/// fail in milliseconds instead of after a round trip to the one that does
/// validate, and so `--threshold` behaves identically between them (review
/// finding on VRN-43).
pub fn validate_threshold_pct(threshold_pct: f64) -> Result<()> {
    ensure!(
        threshold_pct.is_finite() && threshold_pct >= 0.0,
        "threshold must be a finite percentage >= 0, got {threshold_pct}"
    );
    Ok(())
}

/// Whether `unit` is one of the duration units that [`to_nanoseconds`] can
/// convert to a common scale.
pub fn is_duration_unit(unit: &str) -> bool {
    matches!(
        unit,
        "seconds" | "milliseconds" | "microseconds" | "nanoseconds"
    )
}

/// Convert a duration `value` in `unit` to nanoseconds. Non-duration units
/// pass through unchanged.
pub fn to_nanoseconds(value: f64, unit: &str) -> f64 {
    match unit {
        "seconds" => value * 1_000_000_000.0,
        "milliseconds" => value * 1_000_000.0,
        "microseconds" => value * 1_000.0,
        "nanoseconds" => value,
        _ => value,
    }
}

/// Unit families that are higher-is-better: throughput-like units, plus
/// `ratio`. Everything else (time, bytes, allocations, cache/branch misses,
/// instructions, cycles, page_faults, dimensionless) is lower-is-better.
pub fn metric_lower_is_better(unit: &str) -> bool {
    !matches!(unit, "count_per_second" | "bits_per_second" | "ratio")
}

/// Identifies one benchmark series. `branch` is carried for display only —
/// see [`compare`]'s doc comment for why it is not part of series identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesKey {
    pub benchmark_name: String,
    pub metric_name: String,
    /// The stored parameter bindings, e.g. `{"size":100}`, or `"{}"` for a
    /// non-parameterized benchmark.
    pub parameters_json: String,
    pub branch: String,
}

/// A series' mean at a single commit, the input to [`compare`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesMean {
    pub key: SeriesKey,
    pub unit: String,
    pub mean: f64,
}

/// Classification of a compared series' change, relative to `threshold_pct`
/// and the unit's higher-is-better/lower-is-better direction
/// ([`metric_lower_is_better`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Regressed,
    Improved,
    Unchanged,
}

/// What happened to one series between base and head. A sum type rather
/// than a bag of `Option`s, so e.g. "added" can't carry a `base_mean` and
/// "incomparable" can't carry a percentage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Present at both commits, on a common unit scale.
    Compared {
        unit: String,
        base_mean: f64,
        head_mean: f64,
        diff: f64,
        /// `None` when `base_mean` is zero: a true 0-to-0 non-change, or an
        /// unbounded change whose direction is recoverable from `diff`'s
        /// sign.
        diff_pct: Option<f64>,
        verdict: Verdict,
    },
    /// Present at head only.
    Added { unit: String, head_mean: f64 },
    /// Present at base only.
    Removed { unit: String, base_mean: f64 },
    /// Present at both commits, but on units with no common scale (e.g.
    /// bytes vs ratio).
    Incomparable {
        base_unit: String,
        head_unit: String,
    },
    /// At least one side's stored mean is not a valid measurement (NaN or
    /// +-infinity).
    Invalid { reason: String },
}

/// One series' outcome, see [`compare`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub key: SeriesKey,
    pub outcome: Outcome,
}

/// Headline counts for a [`Report`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub regressed: u32,
    pub improved: u32,
    pub unchanged: u32,
    pub added: u32,
    pub removed: u32,
    /// Count of `Incomparable`/`Invalid` series: present at both commits but
    /// with no verdict. Not folded into `unchanged`.
    pub unassessable: u32,
    pub threshold_pct: f64,
    pub has_regressions: bool,
}

/// The result of [`compare`]: every series' outcome plus a headline summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub comparisons: Vec<Comparison>,
    pub summary: Summary,
}

/// Identity used to match a series between `base` and `head`: everything in
/// [`SeriesKey`] except `branch`.
type SeriesIdentity = (String, String, String);

fn identity(key: &SeriesKey) -> SeriesIdentity {
    (
        key.benchmark_name.clone(),
        key.metric_name.clone(),
        key.parameters_json.clone(),
    )
}

/// Compare `base` and `head` series means and classify each one against
/// `threshold_pct`. Mirrors `regression::compare` in rafn-backend field for
/// field — see that crate's doc comments for the full rationale (branch kept
/// out of series identity for cross-branch PR comparisons, duration unit
/// normalization, zero-baseline handling, non-finite guarding).
pub fn compare(base: &[SeriesMean], head: &[SeriesMean], threshold_pct: f64) -> Report {
    let base_by_identity: HashMap<SeriesIdentity, &SeriesMean> =
        base.iter().map(|m| (identity(&m.key), m)).collect();
    let head_by_identity: HashMap<SeriesIdentity, &SeriesMean> =
        head.iter().map(|m| (identity(&m.key), m)).collect();

    let mut comparisons: Vec<Comparison> = Vec::with_capacity(base.len() + head.len());

    // Over `*_by_identity`'s values, not `head`/`base` directly: a results
    // directory can contain the same benchmark/metric/parameters identity in
    // more than one `BenchmarkSet`, and the maps above already collapsed
    // those to one entry per identity. Iterating the raw slices instead used
    // to emit one comparison per duplicate, each matched against whichever
    // value the *other* side's map happened to retain - duplicate rows and
    // inflated summary counts (review finding on VRN-43).
    for head_mean in head_by_identity.values() {
        match base_by_identity.get(&identity(&head_mean.key)) {
            Some(base_mean) => comparisons.push(compare_pair(base_mean, head_mean, threshold_pct)),
            None => comparisons.push(Comparison {
                key: head_mean.key.clone(),
                outcome: if head_mean.mean.is_finite() {
                    Outcome::Added {
                        unit: head_mean.unit.clone(),
                        head_mean: head_mean.mean,
                    }
                } else {
                    Outcome::Invalid {
                        reason: "head mean is non-finite".to_string(),
                    }
                },
            }),
        }
    }

    for base_mean in base_by_identity.values() {
        if !head_by_identity.contains_key(&identity(&base_mean.key)) {
            comparisons.push(Comparison {
                key: base_mean.key.clone(),
                outcome: if base_mean.mean.is_finite() {
                    Outcome::Removed {
                        unit: base_mean.unit.clone(),
                        base_mean: base_mean.mean,
                    }
                } else {
                    Outcome::Invalid {
                        reason: "base mean is non-finite".to_string(),
                    }
                },
            });
        }
    }

    sort_comparisons(&mut comparisons);
    let summary = summarize(&comparisons, threshold_pct);

    Report {
        comparisons,
        summary,
    }
}

/// The string [`unit_name`] produces for `UNIT_UNSPECIFIED` (a producer that
/// omitted `BenchmarkSet.unit` or sent an unrecognized enum value).
const UNIT_UNSPECIFIED: &str = "unspecified";

fn invalid_comparison(key: SeriesKey, reason: impl Into<String>) -> Comparison {
    Comparison {
        key,
        outcome: Outcome::Invalid {
            reason: reason.into(),
        },
    }
}

fn incomparable_comparison(key: SeriesKey, base_unit: String, head_unit: String) -> Comparison {
    Comparison {
        key,
        outcome: Outcome::Incomparable {
            base_unit,
            head_unit,
        },
    }
}

fn compare_pair(base: &SeriesMean, head: &SeriesMean, threshold_pct: f64) -> Comparison {
    let key = head.key.clone();

    if !base.mean.is_finite() || !head.mean.is_finite() {
        let reason = match (base.mean.is_finite(), head.mean.is_finite()) {
            (false, false) => "base and head means are both non-finite",
            (false, true) => "base mean is non-finite",
            (true, false) => "head mean is non-finite",
            (true, true) => unreachable!(),
        };
        return invalid_comparison(key, reason);
    }

    if base.unit == UNIT_UNSPECIFIED || head.unit == UNIT_UNSPECIFIED {
        return incomparable_comparison(key, base.unit.clone(), head.unit.clone());
    }

    let (unit, base_val, head_val) = if is_duration_unit(&base.unit) && is_duration_unit(&head.unit)
    {
        (
            "nanoseconds".to_string(),
            to_nanoseconds(base.mean, &base.unit),
            to_nanoseconds(head.mean, &head.unit),
        )
    } else if base.unit == head.unit {
        (head.unit.clone(), base.mean, head.mean)
    } else {
        return incomparable_comparison(key, base.unit.clone(), head.unit.clone());
    };

    // The stored means were finite, but normalizing or subtracting two large
    // finite values of opposite sign can still overflow to +-infinity.
    if !base_val.is_finite() || !head_val.is_finite() {
        return invalid_comparison(key, "normalized value overflowed to non-finite");
    }

    let diff = head_val - base_val;
    if !diff.is_finite() {
        return invalid_comparison(key, "diff overflowed to non-finite");
    }

    let diff_pct = if base_val == 0.0 {
        (diff == 0.0).then_some(0.0)
    } else {
        // Divide by `base_val.abs()`, not `base_val`: for a negative
        // baseline, dividing by the signed value would flip the sign of the
        // percentage relative to `diff`'s actual sign.
        let pct = diff / base_val.abs() * 100.0;
        if !pct.is_finite() {
            return invalid_comparison(key, "diff_pct overflowed to non-finite");
        }
        Some(pct)
    };

    let lower_is_better = metric_lower_is_better(&unit);
    let verdict = classify(diff, diff_pct, threshold_pct, lower_is_better);

    Comparison {
        key,
        outcome: Outcome::Compared {
            unit,
            base_mean: base_val,
            head_mean: head_val,
            diff,
            diff_pct,
            verdict,
        },
    }
}

/// `Unchanged` when the change is within `threshold_pct` (or is a true
/// 0-to-0 non-change); otherwise `Regressed`/`Improved` by the sign of the
/// change combined with the unit's higher-is-better/lower-is-better family.
fn classify(
    diff: f64,
    diff_pct: Option<f64>,
    threshold_pct: f64,
    lower_is_better: bool,
) -> Verdict {
    match diff_pct {
        Some(pct) if pct.abs() <= threshold_pct => Verdict::Unchanged,
        Some(pct) => worse_or_better(if lower_is_better { pct } else { -pct }),
        None if diff == 0.0 => Verdict::Unchanged,
        None => worse_or_better(if lower_is_better { diff } else { -diff }),
    }
}

/// `worse` is positive-means-regressed, already accounting for the unit's
/// direction.
fn worse_or_better(worse: f64) -> Verdict {
    if worse > 0.0 {
        Verdict::Regressed
    } else {
        Verdict::Improved
    }
}

fn summarize(comparisons: &[Comparison], threshold_pct: f64) -> Summary {
    let mut summary = Summary {
        regressed: 0,
        improved: 0,
        unchanged: 0,
        added: 0,
        removed: 0,
        unassessable: 0,
        threshold_pct,
        has_regressions: false,
    };

    for comparison in comparisons {
        match &comparison.outcome {
            Outcome::Compared { verdict, .. } => match verdict {
                Verdict::Regressed => summary.regressed += 1,
                Verdict::Improved => summary.improved += 1,
                Verdict::Unchanged => summary.unchanged += 1,
            },
            Outcome::Added { .. } => summary.added += 1,
            Outcome::Removed { .. } => summary.removed += 1,
            Outcome::Incomparable { .. } | Outcome::Invalid { .. } => summary.unassessable += 1,
        }
    }

    summary.has_regressions = summary.regressed > 0;
    summary
}

/// Regressions first, then by descending `|diff_pct|`, then by key.
/// Outcomes with no percentage to rank by (`Added`, `Removed`,
/// `Incomparable`, `Invalid`) sort after every regressed/improved series.
fn sort_comparisons(comparisons: &mut [Comparison]) {
    comparisons.sort_by(|a, b| {
        is_regressed(&b.outcome)
            .cmp(&is_regressed(&a.outcome))
            .then_with(|| {
                magnitude(&b.outcome)
                    .partial_cmp(&magnitude(&a.outcome))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| sort_key(&a.key).cmp(&sort_key(&b.key)))
    });
}

fn is_regressed(outcome: &Outcome) -> bool {
    matches!(
        outcome,
        Outcome::Compared {
            verdict: Verdict::Regressed,
            ..
        }
    )
}

fn magnitude(outcome: &Outcome) -> f64 {
    match outcome {
        Outcome::Compared {
            diff_pct: Some(pct),
            ..
        } => pct.abs(),
        Outcome::Compared { diff_pct: None, .. } => f64::INFINITY,
        Outcome::Added { .. }
        | Outcome::Removed { .. }
        | Outcome::Incomparable { .. }
        | Outcome::Invalid { .. } => 0.0,
    }
}

fn sort_key(key: &SeriesKey) -> (&str, &str, &str, &str) {
    (
        &key.benchmark_name,
        &key.metric_name,
        &key.parameters_json,
        &key.branch,
    )
}

/// Map the proto `Unit` enum to the lowercase string [`compare`] operates on
/// (e.g. `UNIT_NANOSECONDS` -> `"nanoseconds"`), matching the string
/// `db::rows::unit_name` produces on the rafn-backend side.
pub fn unit_name(unit: i32) -> String {
    Unit::try_from(unit)
        .map(|u| u.as_str_name())
        .unwrap_or("UNIT_UNSPECIFIED")
        .strip_prefix("UNIT_")
        .unwrap_or("UNSPECIFIED")
        .to_ascii_lowercase()
}

fn parameter_value_json(value: &crate::proto::pb::ParameterValue) -> serde_json::Value {
    match value.value.as_ref() {
        Some(parameter_value::Value::IntValue(v)) => serde_json::json!(v),
        Some(parameter_value::Value::DoubleValue(v)) => serde_json::json!(v),
        Some(parameter_value::Value::BoolValue(v)) => serde_json::json!(v),
        Some(parameter_value::Value::StringValue(v)) => serde_json::json!(v),
        None => serde_json::Value::Null,
    }
}

/// Canonical JSON for a benchmark's parameter bindings, matching
/// `db::rows::parameters_json` on the rafn-backend side (both rely on
/// `serde_json::Map`'s default `BTreeMap` backing for deterministic,
/// sorted-key output — neither crate enables the `preserve_order` feature).
fn parameters_json(benchmark: &Benchmark) -> String {
    let parameters = benchmark
        .parameters
        .iter()
        .map(|(name, value)| (name.clone(), parameter_value_json(value)))
        .collect::<serde_json::Map<_, _>>();
    serde_json::to_string(&parameters).unwrap_or_else(|_| "{}".to_string())
}

/// Flatten a snapshot's benchmark sets into per-series means, keyed the same
/// way as a `CompareCommits` response, so a local comparison and a remote one
/// read identically.
pub fn flatten_series(sets: &[BenchmarkSet]) -> Vec<SeriesMean> {
    sets.iter()
        .flat_map(|set| {
            let unit = unit_name(set.unit);
            let branch = set
                .source
                .as_ref()
                .and_then(|s| s.branch.clone())
                .unwrap_or_default();
            set.benchmarks.iter().filter_map(move |benchmark| {
                let mean = statistic_mean_ns(benchmark)?;
                Some(SeriesMean {
                    key: SeriesKey {
                        benchmark_name: benchmark.name.clone(),
                        metric_name: set.metric_name.clone(),
                        parameters_json: parameters_json(benchmark),
                        branch: branch.clone(),
                    },
                    unit: unit.clone(),
                    mean,
                })
            })
        })
        .collect()
}

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

/// Colors by `verdict`, not by `pct`'s raw sign: for a higher-is-better unit
/// (`count_per_second`, `bits_per_second`, `ratio`) a regression is a
/// negative percentage, and coloring by sign alone would show it green next
/// to a red `Regressed` verdict (review finding on VRN-43).
pub fn format_percent(pct: &f64, verdict: Verdict) -> String {
    let s = if *pct > 0.0 {
        format!("+{pct:.1}%")
    } else {
        format!("{pct:.1}%")
    };
    match verdict {
        Verdict::Regressed => s.red().to_string(),
        Verdict::Improved => s.green().to_string(),
        Verdict::Unchanged => s,
    }
}

fn format_value(value: f64, unit: &str) -> String {
    if is_duration_unit(unit) {
        // `Added`/`Removed` pass the series' own stored unit (e.g. "seconds"),
        // not the "nanoseconds" `compare_pair` normalizes `Compared` rows to
        // - so unlike those, `value` here is not already in nanoseconds and
        // must go through `to_nanoseconds` first (review finding on VRN-43).
        format_duration(&to_nanoseconds(value, unit))
    } else {
        format!("{value:.3} {unit}")
    }
}

fn format_value_diff(value: f64, unit: &str) -> String {
    if is_duration_unit(unit) {
        format_diff(&value)
    } else {
        let sign = if value > 0.0 { "+" } else { "" };
        format!("{sign}{value:.3} {unit}")
    }
}

fn format_change_pct(pct: Option<f64>, verdict: Verdict) -> String {
    match pct {
        Some(pct) => format_percent(&pct, verdict),
        None => "unbounded".to_string(),
    }
}

fn format_verdict(verdict: Verdict) -> String {
    match verdict {
        Verdict::Regressed => "✗ regressed".red().to_string(),
        Verdict::Improved => "✓ improved".green().to_string(),
        Verdict::Unchanged => "unchanged".to_string(),
    }
}

/// Includes `metric_name`, not just `benchmark_name`: series identity
/// ([`identity`]) is keyed on both, so two metrics of the same benchmark
/// (e.g. wall time and CPU time, common in remote `CompareCommits`
/// responses) are different series and must render as different rows
/// (review finding on VRN-43).
fn display_name(key: &SeriesKey) -> String {
    let name = format!("{} ({})", key.benchmark_name, key.metric_name);
    if key.parameters_json.is_empty() || key.parameters_json == "{}" {
        name
    } else {
        format!("{name} {}", key.parameters_json)
    }
}

#[derive(Debug, Clone, Tabled)]
struct ComparedRow {
    #[tabled(rename = "Benchmark")]
    benchmark: String,
    #[tabled(rename = "Base")]
    base: String,
    #[tabled(rename = "Head")]
    head: String,
    #[tabled(rename = "Diff")]
    diff: String,
    #[tabled(rename = "Change %")]
    change_pct: String,
    #[tabled(rename = "Verdict")]
    verdict: String,
}

/// Print a comparison report to stdout: a table of directly-compared series,
/// a line per added/removed/incomparable/invalid series, and a summary line.
// stdout is this CLI's output contract, not debug noise — users pipe/read it
// directly, unlike `tracing`'s log lines.
#[allow(clippy::print_stdout)]
pub fn print_report(report: &Report) {
    let compared_rows: Vec<ComparedRow> = report
        .comparisons
        .iter()
        .filter_map(|comparison| match &comparison.outcome {
            Outcome::Compared {
                unit,
                base_mean,
                head_mean,
                diff,
                diff_pct,
                verdict,
            } => Some(ComparedRow {
                benchmark: display_name(&comparison.key),
                base: format_value(*base_mean, unit),
                head: format_value(*head_mean, unit),
                diff: format_value_diff(*diff, unit),
                change_pct: format_change_pct(*diff_pct, *verdict),
                verdict: format_verdict(*verdict),
            }),
            _ => None,
        })
        .collect();

    if !compared_rows.is_empty() {
        println!("{}", Table::new(&compared_rows));
        println!();
    }

    for comparison in &report.comparisons {
        let name = display_name(&comparison.key);
        match &comparison.outcome {
            Outcome::Added { unit, head_mean } => {
                println!("+ {name} added: {}", format_value(*head_mean, unit));
            }
            Outcome::Removed { unit, base_mean } => {
                println!("- {name} removed: {}", format_value(*base_mean, unit));
            }
            Outcome::Incomparable {
                base_unit,
                head_unit,
            } => {
                println!("? {name} incomparable: {base_unit} vs {head_unit}");
            }
            Outcome::Invalid { reason } => {
                println!("! {name} invalid: {reason}");
            }
            Outcome::Compared { .. } => {}
        }
    }

    println!();
    let s = &report.summary;
    println!(
        "{} regressed, {} improved, {} unchanged, {} added, {} removed, {} unassessable (threshold {:.2}%)",
        s.regressed, s.improved, s.unchanged, s.added, s.removed, s.unassessable, s.threshold_pct
    );
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
        assert_eq!(format_diff(&2_500_000_000.0), "+2.500 s");
    }

    #[test]
    fn format_diff_no_plus_for_negative_or_zero() {
        assert_eq!(format_diff(&-1_149.582), "-1.150 µs");
        assert_eq!(format_diff(&0.0), "0.000 ns");
        assert_eq!(format_diff(&-2_500_000_000.0), "-2.500 s");
    }

    #[test]
    fn format_percent_regressed_is_red_with_plus_for_positive_pct() {
        assert_eq!(
            strip_ansi(&format_percent(&10.0, Verdict::Regressed)),
            "+10.0%"
        );
    }

    #[test]
    fn format_percent_improved_is_green_without_plus_for_negative_pct() {
        assert_eq!(
            strip_ansi(&format_percent(&-7.5, Verdict::Improved)),
            "-7.5%"
        );
    }

    #[test]
    fn format_percent_unchanged_is_plain() {
        let out = format_percent(&0.0, Verdict::Unchanged);
        assert_eq!(out, "0.0%");
        assert!(
            !out.contains("\x1b["),
            "an unchanged verdict should have no ANSI codes"
        );
    }

    #[test]
    fn format_percent_colors_by_verdict_not_by_raw_sign() {
        // Forced on: `colored` no-ops in this (non-tty) test environment by
        // default, which would make every branch below produce identical
        // plain text and defeat the assertions regardless of which is
        // correct. No other test in this crate touches `colored::control`,
        // so this doesn't race.
        colored::control::set_override(true);

        // Same magnitude and sign, opposite verdicts - e.g. a throughput
        // regression (higher-is-better, so a drop is a negative diff_pct)
        // versus a duration improvement (lower-is-better, so a drop is also
        // a negative diff_pct but a good thing). Coloring by sign alone
        // would render both identically green (review finding on VRN-43).
        let regressed = format_percent(&-20.0, Verdict::Regressed);
        let improved = format_percent(&-20.0, Verdict::Improved);
        assert_eq!(regressed, "-20.0%".red().to_string());
        assert_eq!(improved, "-20.0%".green().to_string());
        assert_ne!(regressed, improved);

        colored::control::unset_override();
    }

    #[test]
    fn validate_threshold_pct_accepts_finite_non_negative_values() {
        assert!(validate_threshold_pct(0.0).is_ok());
        assert!(validate_threshold_pct(5.0).is_ok());
    }

    #[test]
    fn validate_threshold_pct_rejects_negative() {
        // A negative threshold would fail even an exact 0% change: `classify`
        // checks `pct.abs() <= threshold_pct`, and `abs()` is never negative.
        assert!(validate_threshold_pct(-0.1).is_err());
    }

    #[test]
    fn validate_threshold_pct_rejects_non_finite() {
        assert!(validate_threshold_pct(f64::NAN).is_err());
        assert!(validate_threshold_pct(f64::INFINITY).is_err());
    }

    fn mean(benchmark: &str, unit: &str, mean: f64) -> SeriesMean {
        mean_ex(benchmark, "wall_time", "", "main", unit, mean)
    }

    fn mean_ex(
        benchmark: &str,
        metric: &str,
        parameters_json: &str,
        branch: &str,
        unit: &str,
        mean: f64,
    ) -> SeriesMean {
        SeriesMean {
            key: SeriesKey {
                benchmark_name: benchmark.to_string(),
                metric_name: metric.to_string(),
                parameters_json: parameters_json.to_string(),
                branch: branch.to_string(),
            },
            unit: unit.to_string(),
            mean,
        }
    }

    fn compared<'a>(
        report: &'a Report,
        benchmark: &str,
    ) -> (&'a str, f64, f64, f64, Option<f64>, Verdict) {
        let Some(comparison) = report
            .comparisons
            .iter()
            .find(|c| c.key.benchmark_name == benchmark)
        else {
            unreachable!("{benchmark} present in report");
        };
        match &comparison.outcome {
            Outcome::Compared {
                unit,
                base_mean,
                head_mean,
                diff,
                diff_pct,
                verdict,
            } => (unit, *base_mean, *head_mean, *diff, *diff_pct, *verdict),
            other => unreachable!("{benchmark} expected Compared, got {other:?}"),
        }
    }

    #[test]
    fn regression_above_threshold_is_flagged() {
        let base = vec![mean("bench", "nanoseconds", 100.0)];
        let head = vec![mean("bench", "nanoseconds", 120.0)];

        let report = compare(&base, &head, 5.0);

        let (_, _, _, _, diff_pct, verdict) = compared(&report, "bench");
        assert_eq!(verdict, Verdict::Regressed);
        assert_eq!(diff_pct, Some(20.0));
        assert!(report.summary.has_regressions);
        assert_eq!(report.summary.regressed, 1);
    }

    #[test]
    fn improvement_is_flagged() {
        let base = vec![mean("bench", "nanoseconds", 100.0)];
        let head = vec![mean("bench", "nanoseconds", 80.0)];

        let report = compare(&base, &head, 5.0);

        let (_, _, _, _, _, verdict) = compared(&report, "bench");
        assert_eq!(verdict, Verdict::Improved);
        assert_eq!(report.summary.improved, 1);
        assert!(!report.summary.has_regressions);
    }

    #[test]
    fn noise_inside_threshold_is_unchanged() {
        let base = vec![mean("bench", "nanoseconds", 100.0)];
        let head = vec![mean("bench", "nanoseconds", 102.0)];

        let report = compare(&base, &head, 5.0);

        let (_, _, _, _, _, verdict) = compared(&report, "bench");
        assert_eq!(verdict, Verdict::Unchanged);
        assert_eq!(report.summary.unchanged, 1);
    }

    #[test]
    fn exactly_at_threshold_is_unchanged() {
        let base = vec![mean("bench", "nanoseconds", 100.0)];
        let head = vec![mean("bench", "nanoseconds", 105.0)];

        let report = compare(&base, &head, 5.0);

        let (_, _, _, _, diff_pct, verdict) = compared(&report, "bench");
        assert_eq!(diff_pct, Some(5.0));
        assert_eq!(verdict, Verdict::Unchanged, "boundary is inclusive");
    }

    #[test]
    fn benchmark_only_at_head_is_added() {
        let base: Vec<SeriesMean> = vec![];
        let head = vec![mean("bench", "nanoseconds", 100.0)];

        let report = compare(&base, &head, 5.0);

        assert_eq!(report.comparisons.len(), 1);
        match &report.comparisons[0].outcome {
            Outcome::Added { unit, head_mean } => {
                assert_eq!(unit, "nanoseconds");
                assert_eq!(*head_mean, 100.0);
            }
            other => unreachable!("expected Added, got {other:?}"),
        }
        assert_eq!(report.summary.added, 1);
    }

    #[test]
    fn benchmark_only_at_base_is_removed() {
        let base = vec![mean("bench", "nanoseconds", 100.0)];
        let head: Vec<SeriesMean> = vec![];

        let report = compare(&base, &head, 5.0);

        assert_eq!(report.comparisons.len(), 1);
        match &report.comparisons[0].outcome {
            Outcome::Removed { unit, base_mean } => {
                assert_eq!(unit, "nanoseconds");
                assert_eq!(*base_mean, 100.0);
            }
            other => unreachable!("expected Removed, got {other:?}"),
        }
        assert_eq!(report.summary.removed, 1);
    }

    #[test]
    fn seconds_and_milliseconds_normalize_and_compare() {
        let base = vec![mean("bench", "seconds", 1.0)];
        let head = vec![mean("bench", "milliseconds", 1000.0)];

        let report = compare(&base, &head, 5.0);

        let (unit, base_mean, head_mean, diff, diff_pct, verdict) = compared(&report, "bench");
        assert_eq!(unit, "nanoseconds");
        assert_eq!(base_mean, 1_000_000_000.0);
        assert_eq!(head_mean, 1_000_000_000.0);
        assert_eq!(diff, 0.0);
        assert_eq!(diff_pct, Some(0.0));
        assert_eq!(verdict, Verdict::Unchanged);
    }

    #[test]
    fn bytes_and_ratio_are_incomparable() {
        let base = vec![mean("bench", "bytes", 100.0)];
        let head = vec![mean("bench", "ratio", 1.5)];

        let report = compare(&base, &head, 5.0);

        assert_eq!(report.comparisons.len(), 1);
        assert!(matches!(
            report.comparisons[0].outcome,
            Outcome::Incomparable { .. }
        ));
        assert_eq!(report.summary.unassessable, 1);
    }

    #[test]
    fn unspecified_unit_never_issues_a_verdict() {
        let base = vec![mean("bench", "unspecified", 100.0)];
        let head = vec![mean("bench", "unspecified", 200.0)];

        let report = compare(&base, &head, 5.0);

        assert!(matches!(
            report.comparisons[0].outcome,
            Outcome::Incomparable { .. }
        ));
        assert_eq!(report.summary.regressed, 0);
        assert_eq!(report.summary.unassessable, 1);
    }

    #[test]
    fn non_finite_means_are_invalid_not_a_verdict() {
        let base = vec![mean("bench", "nanoseconds", f64::NAN)];
        let head = vec![mean("bench", "nanoseconds", 100.0)];

        let report = compare(&base, &head, 5.0);

        assert!(matches!(
            report.comparisons[0].outcome,
            Outcome::Invalid { .. }
        ));
        assert_eq!(report.summary.unassessable, 1);
    }

    #[test]
    fn negative_baseline_percentage_matches_diff_sign() {
        let base = vec![mean("bench", "dimensionless", -100.0)];
        let head = vec![mean("bench", "dimensionless", -50.0)];

        let report = compare(&base, &head, 5.0);

        match &report.comparisons[0].outcome {
            Outcome::Compared { diff, diff_pct, .. } => {
                assert_eq!(*diff, 50.0);
                assert_eq!(*diff_pct, Some(50.0));
            }
            other => unreachable!("expected Compared, got {other:?}"),
        }
    }

    #[test]
    fn zero_baseline_with_nonzero_head_is_unbounded_regression() {
        let base = vec![mean("bench", "allocation_count", 0.0)];
        let head = vec![mean("bench", "allocation_count", 5.0)];

        let report = compare(&base, &head, 5.0);

        let (_, _, _, diff, diff_pct, verdict) = compared(&report, "bench");
        assert_eq!(diff_pct, None);
        assert_eq!(diff, 5.0);
        assert_eq!(verdict, Verdict::Regressed);
    }

    #[test]
    fn zero_to_zero_is_unchanged() {
        let base = vec![mean("bench", "allocation_count", 0.0)];
        let head = vec![mean("bench", "allocation_count", 0.0)];

        let report = compare(&base, &head, 5.0);

        let (_, _, _, _, diff_pct, verdict) = compared(&report, "bench");
        assert_eq!(diff_pct, Some(0.0));
        assert_eq!(verdict, Verdict::Unchanged);
    }

    #[test]
    fn count_per_second_drop_is_the_regression() {
        let base = vec![mean("bench", "count_per_second", 1000.0)];
        let head = vec![mean("bench", "count_per_second", 800.0)];

        let report = compare(&base, &head, 5.0);

        let (_, _, _, _, _, verdict) = compared(&report, "bench");
        assert_eq!(
            verdict,
            Verdict::Regressed,
            "a throughput drop is a regression"
        );

        let base = vec![mean("bench", "count_per_second", 1000.0)];
        let head = vec![mean("bench", "count_per_second", 1200.0)];
        let report = compare(&base, &head, 5.0);
        let (_, _, _, _, _, verdict) = compared(&report, "bench");
        assert_eq!(
            verdict,
            Verdict::Improved,
            "a throughput rise is an improvement"
        );
    }

    #[test]
    fn count_per_second_regression_change_pct_renders_red_despite_negative_diff_pct() {
        let base = vec![mean("bench", "count_per_second", 1000.0)];
        let head = vec![mean("bench", "count_per_second", 800.0)];
        let report = compare(&base, &head, 5.0);

        let (_, _, _, _, diff_pct, verdict) = compared(&report, "bench");
        assert_eq!(verdict, Verdict::Regressed);
        let diff_pct = diff_pct.unwrap_or_else(|| unreachable!("nonzero base means a percentage"));
        assert!(diff_pct < 0.0, "a throughput drop is a negative diff_pct");

        let rendered = format_change_pct(Some(diff_pct), verdict);
        assert_eq!(
            rendered,
            format!("{diff_pct:.1}%").red().to_string(),
            "a regression must render red even though diff_pct is negative"
        );
    }

    #[test]
    fn duplicate_series_identity_in_head_produces_one_comparison() {
        let base = vec![mean("bench", "nanoseconds", 100.0)];
        // Stands in for a results directory containing the same
        // benchmark/metric/parameters identity in more than one
        // `BenchmarkSet`.
        let head = vec![
            mean("bench", "nanoseconds", 120.0),
            mean("bench", "nanoseconds", 150.0),
        ];

        let report = compare(&base, &head, 5.0);

        assert_eq!(
            report.comparisons.len(),
            1,
            "a duplicate head identity must not produce duplicate rows"
        );
    }

    #[test]
    fn duplicate_series_identity_in_base_produces_one_comparison() {
        let base = vec![
            mean("bench", "nanoseconds", 100.0),
            mean("bench", "nanoseconds", 110.0),
        ];
        let head: Vec<SeriesMean> = vec![];

        let report = compare(&base, &head, 5.0);

        assert_eq!(
            report.comparisons.len(),
            1,
            "a duplicate base identity must not produce duplicate removed rows"
        );
    }

    #[test]
    fn display_name_includes_metric_to_disambiguate_same_benchmark_different_metrics() {
        let wall = SeriesKey {
            benchmark_name: "bench".to_string(),
            metric_name: "wall_time".to_string(),
            parameters_json: String::new(),
            branch: "main".to_string(),
        };
        let cpu = SeriesKey {
            metric_name: "cpu_time".to_string(),
            ..wall.clone()
        };

        assert_eq!(display_name(&wall), "bench (wall_time)");
        assert_eq!(display_name(&cpu), "bench (cpu_time)");
        assert_ne!(display_name(&wall), display_name(&cpu));
    }

    #[test]
    fn display_name_keeps_parameters_after_the_metric() {
        let key = SeriesKey {
            benchmark_name: "sort".to_string(),
            metric_name: "wall_time".to_string(),
            parameters_json: r#"{"size":100}"#.to_string(),
            branch: "main".to_string(),
        };

        assert_eq!(display_name(&key), r#"sort (wall_time) {"size":100}"#);
    }

    #[test]
    fn format_value_converts_a_non_normalized_duration_unit_before_scaling() {
        // `Added`/`Removed` pass the series' own unit, not the "nanoseconds"
        // `compare_pair` normalizes `Compared` rows to - 1.0 seconds must
        // not print as "1.000 ns".
        assert_eq!(format_value(1.0, "seconds"), "1.000 s");
        assert_eq!(format_value(1.0, "milliseconds"), "1.000 ms");
    }

    #[test]
    fn parameter_bindings_keep_series_separate() {
        let base = vec![
            mean_ex(
                "sort",
                "wall_time",
                r#"{"size":100}"#,
                "main",
                "nanoseconds",
                100.0,
            ),
            mean_ex(
                "sort",
                "wall_time",
                r#"{"size":1000}"#,
                "main",
                "nanoseconds",
                2000.0,
            ),
        ];
        let head = vec![
            mean_ex(
                "sort",
                "wall_time",
                r#"{"size":100}"#,
                "main",
                "nanoseconds",
                200.0,
            ),
            mean_ex(
                "sort",
                "wall_time",
                r#"{"size":1000}"#,
                "main",
                "nanoseconds",
                1000.0,
            ),
        ];

        let report = compare(&base, &head, 5.0);

        assert_eq!(report.comparisons.len(), 2);
        let size_100 = report
            .comparisons
            .iter()
            .find(|c| c.key.parameters_json == r#"{"size":100}"#)
            .unwrap_or_else(|| unreachable!("size=100 binding present"));
        match &size_100.outcome {
            Outcome::Compared { verdict, .. } => assert_eq!(*verdict, Verdict::Regressed),
            other => unreachable!("expected Compared, got {other:?}"),
        }
    }

    #[test]
    fn cross_branch_comparison_still_matches_and_carries_head_branch() {
        let base = vec![mean_ex(
            "bench",
            "wall_time",
            "",
            "main",
            "nanoseconds",
            100.0,
        )];
        let head = vec![mean_ex(
            "bench",
            "wall_time",
            "",
            "feature/x",
            "nanoseconds",
            200.0,
        )];

        let report = compare(&base, &head, 5.0);

        assert_eq!(
            report.comparisons.len(),
            1,
            "branch is not part of series identity"
        );
        assert_eq!(report.comparisons[0].key.branch, "feature/x");
    }

    #[test]
    fn ordering_puts_regressions_first() {
        let base = vec![
            mean("improved", "nanoseconds", 100.0),
            mean("regressed_small", "nanoseconds", 100.0),
            mean("regressed_big", "nanoseconds", 100.0),
        ];
        let head = vec![
            mean("improved", "nanoseconds", 50.0),
            mean("regressed_small", "nanoseconds", 110.0),
            mean("regressed_big", "nanoseconds", 200.0),
        ];

        let report = compare(&base, &head, 5.0);

        let names: Vec<&str> = report
            .comparisons
            .iter()
            .map(|c| c.key.benchmark_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["regressed_big", "regressed_small", "improved"],
            "regressions first, worst first"
        );
    }

    #[test]
    fn report_serializes_null_diff_pct_and_round_trips() {
        let base = vec![mean("bench", "allocation_count", 0.0)];
        let head = vec![mean("bench", "allocation_count", 5.0)];
        let report = compare(&base, &head, 5.0);

        let json = serde_json::to_string(&report)
            .unwrap_or_else(|e| unreachable!("must serialize as valid JSON: {e}"));
        assert!(json.contains("\"diff_pct\":null"));

        let round_tripped: Report = serde_json::from_str(&json)
            .unwrap_or_else(|e| unreachable!("must deserialize back without error: {e}"));
        assert_eq!(round_tripped.comparisons.len(), report.comparisons.len());
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
            Some("main".to_string()),
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
    fn flatten_series_keys_on_name_metric_and_carries_branch_and_unit() {
        let sets = vec![make_set("foo", 1_000_000.0)];
        let series = flatten_series(&sets);

        assert_eq!(series.len(), 1);
        assert_eq!(series[0].key.benchmark_name, "foo");
        assert_eq!(series[0].key.metric_name, "wall_time");
        assert_eq!(series[0].key.parameters_json, "{}");
        assert_eq!(series[0].key.branch, "main");
        assert_eq!(series[0].unit, "nanoseconds");
        assert_eq!(series[0].mean, 1_000_000.0);
    }

    #[test]
    fn flatten_series_skips_benchmarks_without_a_mean() {
        let mut sets = vec![make_set("foo", 1_000_000.0)];
        sets[0].benchmarks[0].statistics = None;

        assert!(flatten_series(&sets).is_empty());
    }

    #[test]
    fn compare_end_to_end_over_flattened_snapshots() {
        let base = vec![make_set("foo", 1_000_000.0)];
        let head = vec![make_set("foo", 1_100_000.0)];

        let report = compare(&flatten_series(&base), &flatten_series(&head), 5.0);

        let (_, _, _, _, diff_pct, verdict) = compared(&report, "foo");
        assert_eq!(verdict, Verdict::Regressed);
        assert!((diff_pct.unwrap_or_default() - 10.0).abs() < 0.01);
    }
}
