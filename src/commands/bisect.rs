//! `rafn bisect` — find the commit that introduced a performance regression.
//!
//! Phase 1 benchmarks `--good` and `--bad` directly, both to confirm there is
//! a regression worth hunting and to capture the baseline. Phase 2 hands the
//! search to `git bisect run`, which re-invokes this binary as the hidden
//! `rafn bisect-step` subcommand at every candidate commit. The step is a
//! separate process running under a checkout that keeps changing, so
//! everything it needs is persisted in `rafn/bisect/` under the worktree's
//! git directory, where a `git clean -fdx` build step can't delete it.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::comparison::{self, Outcome, Report, SeriesMean, Verdict};
use crate::config::{Config, EffectiveConfig, RepoConfig, RepositoryRef};
use crate::framework::{self, ResultsStrategy};
use crate::proto::benchmark::timestamp_now;
use crate::{discovery, git, ingest, runner};

/// Resolved with `git rev-parse --git-path`, i.e. inside `.git/`.
const STATE_DIR: &str = "rafn/bisect";
const SESSION_FILE: &str = "session.json";
const PLAN_FILE: &str = "plan.json";
const STEPS_DIR: &str = "steps";

#[derive(Args, Debug)]
pub struct BisectCommand {
    /// Commit or ref where performance is acceptable
    #[arg(long, required_unless_present = "reset", conflicts_with = "reset")]
    good: Option<String>,

    /// Commit or ref where the regression is present
    #[arg(long, required_unless_present = "reset", conflicts_with = "reset")]
    bad: Option<String>,

    /// Only track benchmarks whose name contains this substring
    #[arg(long, conflicts_with = "reset")]
    benchmark: Option<String>,

    /// Regression threshold percentage (overrides rafn.toml [bench].threshold)
    #[arg(long, conflicts_with = "reset")]
    threshold: Option<f64>,

    /// Benchmark runs per commit; series means are averaged across runs
    #[arg(long, default_value = "1", conflicts_with = "reset")]
    runs: NonZeroU32,

    /// Shell command that builds the project before each commit is benchmarked
    #[arg(long, conflicts_with = "reset")]
    build_cmd: Option<String>,

    /// Abandon an interrupted bisect: restore the original checkout and
    /// remove rafn's temporary bisect state
    #[arg(long)]
    reset: bool,

    /// Arguments passed to the detected benchmark framework command
    #[arg(last = true, conflicts_with = "reset")]
    args: Vec<String>,
}

/// Exit codes of `rafn bisect`, part of its CLI contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BisectExit {
    Found,
    NoRegression,
    DirtyTree,
    Failed,
}

impl From<BisectExit> for ExitCode {
    fn from(exit: BisectExit) -> Self {
        ExitCode::from(match exit {
            BisectExit::Found => 0,
            BisectExit::NoRegression => 1,
            BisectExit::DirtyTree => 2,
            BisectExit::Failed => 3,
        })
    }
}

impl BisectCommand {
    pub fn execute(self) -> ExitCode {
        let outcome = if self.reset {
            reset().map(|()| ExitCode::SUCCESS)
        } else {
            self.into_request().and_then(run).map(ExitCode::from)
        };
        outcome.unwrap_or_else(|err| {
            error!("{err:#}");
            BisectExit::Failed.into()
        })
    }

    fn into_request(self) -> Result<Request> {
        Ok(Request {
            good: self.good.context("--good is required")?,
            bad: self.bad.context("--bad is required")?,
            threshold: self.threshold,
            spec: MeasureSpec {
                work_dir: std::env::current_dir()?,
                benchmark_filter: self.benchmark,
                runs: self.runs,
                build_cmd: self.build_cmd,
                bench_args: self.args,
            },
        })
    }
}

/// Hidden subcommand that `git bisect run` invokes at each candidate commit.
#[derive(Args, Debug)]
pub struct BisectStepCommand {
    /// Plan written by `rafn bisect`
    #[arg(long)]
    plan: PathBuf,
}

/// Exit codes `git bisect run` interprets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepVerdict {
    Good,
    Bad,
    Skip,
    /// Rafn's own state is unusable; any code >= 128 makes `git bisect run`
    /// stop instead of misclassifying every remaining commit.
    Abort,
}

impl From<StepVerdict> for ExitCode {
    fn from(verdict: StepVerdict) -> Self {
        ExitCode::from(match verdict {
            StepVerdict::Good => 0,
            StepVerdict::Bad => 1,
            StepVerdict::Skip => 125,
            StepVerdict::Abort => 128,
        })
    }
}

impl BisectStepCommand {
    pub fn execute(self) -> ExitCode {
        step(&self.plan).into()
    }
}

#[derive(Debug)]
struct Request {
    good: String,
    bad: String,
    threshold: Option<f64>,
    spec: MeasureSpec,
}

/// How to benchmark one commit. Persisted in the plan so every bisect step
/// measures exactly like the baseline did.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MeasureSpec {
    work_dir: PathBuf,
    benchmark_filter: Option<String>,
    runs: NonZeroU32,
    build_cmd: Option<String>,
    bench_args: Vec<String>,
}

/// Where HEAD was before rafn started checking out commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum OriginalHead {
    Branch(String),
    Detached(String),
}

/// Written before the first checkout so `rafn bisect --reset` can restore the
/// user's checkout even if rafn dies during phase 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Session {
    original_head: OriginalHead,
    /// Set once rafn runs `git bisect start`, so a later reset can tell
    /// rafn's bisect apart from one the user started by hand.
    #[serde(default)]
    bisect: Option<BisectRange>,
    /// Checkouts detach submodules; those that were on a branch go back to it.
    #[serde(default)]
    submodule_heads: Vec<SubmoduleHead>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubmoduleHead {
    /// Relative to the superproject root.
    path: String,
    head: OriginalHead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BisectRange {
    bad: String,
    good: String,
}

/// Everything `rafn bisect-step` needs, written once the baseline is known.
#[derive(Debug, Serialize, Deserialize)]
struct Plan {
    git_root: PathBuf,
    spec: MeasureSpec,
    threshold_pct: f64,
    repository: RepositoryRef,
    /// Only the series that regressed between good and bad: other series are
    /// noise for this search and would let an unrelated fluctuation flip a
    /// step's verdict.
    baseline: Vec<SeriesMean>,
    bad_commit: String,
    bad_measurement: Vec<SeriesMean>,
}

fn run(request: Request) -> Result<BisectExit> {
    let repo = Repo::discover()?;
    ensure!(
        !repo.bisect_in_progress()?,
        "A git bisect is already in progress. Finish it with `git bisect reset`, \
         or run `rafn bisect --reset` if rafn started it."
    );
    if repo.is_dirty()? {
        error!("Working tree has uncommitted changes; commit or stash them before bisecting.");
        return Ok(BisectExit::DirtyTree);
    }

    let good = repo.resolve_commit(&request.good)?;
    let bad = repo.resolve_commit(&request.bad)?;
    ensure!(good != bad, "--good and --bad resolve to the same commit");

    let repo_config = RepoConfig::load()?;
    let effective = EffectiveConfig::resolve(&repo_config, &Config::load()?);
    let threshold_pct = request.threshold.unwrap_or(effective.bench_threshold);
    comparison::validate_threshold_pct(threshold_pct)?;
    let repository = effective.repository.unwrap_or_else(unpublished_repository);

    let state_dir = repo.state_dir()?;
    let session_path = state_dir.join(SESSION_FILE);
    // An interrupted session holds the only record of the user's original
    // checkout; starting over would replace it with a detached endpoint.
    ensure!(
        !session_path.exists(),
        "An interrupted rafn bisect session exists. Run `rafn bisect --reset` first."
    );
    let collisions = repo.ignored_collisions(&good, &bad)?;
    ensure!(
        collisions.is_empty(),
        "These ignored paths are tracked by commits in the bisect range, so checking \
         those commits out would overwrite them: {}. Move them aside and retry.",
        collisions.join(", ")
    );
    if state_dir.exists() {
        std::fs::remove_dir_all(&state_dir)
            .with_context(|| format!("Failed to clear stale {}", state_dir.display()))?;
    }
    let session = Session {
        original_head: repo.head()?,
        bisect: None,
        submodule_heads: repo.submodule_heads()?,
    };
    write_json(&session_path, &session)?;

    let inputs = SessionInputs {
        repo: &repo,
        state_dir: &state_dir,
        session: session.clone(),
        good,
        bad,
        threshold_pct,
        repository,
        spec: request.spec,
    };
    let outcome = run_session(inputs);
    // Re-read: `run_session` records the bisect it starts in the session file.
    let session = read_json(&session_path).unwrap_or(session);
    let cleanup = cleanup(&repo, &state_dir, &session);
    match (outcome, cleanup) {
        (Ok(exit), Ok(())) => Ok(exit),
        (Ok(_), Err(err)) => Err(err),
        (Err(err), cleanup) => {
            if let Err(cleanup_err) = cleanup {
                error!("Cleanup also failed: {cleanup_err:#}. Run `rafn bisect --reset`.");
            }
            Err(err)
        }
    }
}

#[derive(Debug)]
struct SessionInputs<'a> {
    repo: &'a Repo,
    state_dir: &'a Path,
    session: Session,
    good: String,
    bad: String,
    threshold_pct: f64,
    repository: RepositoryRef,
    spec: MeasureSpec,
}

fn run_session(inputs: SessionInputs<'_>) -> Result<BisectExit> {
    let SessionInputs {
        repo,
        state_dir,
        mut session,
        good,
        bad,
        threshold_pct,
        repository,
        spec,
    } = inputs;

    info!("Benchmarking good commit {good}");
    repo.checkout_detached(&good)?;
    let good_series = measure(&spec, &good, &repository);
    repo.discard_tracked_changes()?;
    let good_series =
        good_series.with_context(|| format!("Failed to benchmark good commit {good}"))?;

    info!("Benchmarking bad commit {bad}");
    repo.checkout_detached(&bad)?;
    let bad_series = measure(&spec, &bad, &repository);
    repo.discard_tracked_changes()?;
    let bad_series = bad_series.with_context(|| format!("Failed to benchmark bad commit {bad}"))?;

    // `git bisect reset` returns to whatever HEAD was at `git bisect start`,
    // so start from the user's own checkout rather than from `bad`.
    repo.restore(&session.original_head)?;

    let report = comparison::compare(&good_series, &bad_series, threshold_pct);
    comparison::print_report(&report);
    let baseline = regressed_baseline(&good_series, &report);
    if baseline.is_empty() {
        info!(
            "No regression between {good} and {bad} (threshold: {threshold_pct:.1}%) — nothing to bisect."
        );
        return Ok(BisectExit::NoRegression);
    }

    let plan_path = state_dir.join(PLAN_FILE);
    write_json(
        &plan_path,
        &Plan {
            git_root: repo.root.clone(),
            spec,
            threshold_pct,
            repository,
            baseline,
            bad_commit: bad.clone(),
            bad_measurement: bad_series,
        },
    )?;

    session.bisect = Some(BisectRange {
        bad: bad.clone(),
        good: good.clone(),
    });
    write_json(&state_dir.join(SESSION_FILE), &session)?;
    repo.git(&["bisect", "start", &bad, &good])?;
    let exe = std::env::current_exe().context("Failed to locate the rafn executable")?;
    let status = repo
        .command()
        .args(["bisect", "run"])
        .arg(exe)
        .arg("bisect-step")
        .arg("--plan")
        .arg(&plan_path)
        .status()
        .context("Failed to run `git bisect run`")?;
    ensure!(
        status.success(),
        "`git bisect run` did not identify a first bad commit ({status}); \
         too many commits may have been skipped"
    );

    let first_bad = repo.git(&["rev-parse", "--verify", "refs/bisect/bad"])?;
    let plan: Plan = read_json(&plan_path)?;
    let culprit_series = if first_bad == plan.bad_commit {
        plan.bad_measurement
    } else {
        read_json(&step_path(state_dir, &first_bad))?
    };
    let culprit_report = comparison::compare(&plan.baseline, &culprit_series, threshold_pct);
    print_culprit(&repo.describe(&first_bad)?, &culprit_report);

    Ok(BisectExit::Found)
}

fn step(plan_path: &Path) -> StepVerdict {
    let plan: Plan = match read_json(plan_path) {
        Ok(plan) => plan,
        Err(err) => {
            error!("{err:#}");
            return StepVerdict::Abort;
        }
    };
    let repo = Repo {
        root: plan.git_root.clone(),
    };
    let commit = match repo.git(&["rev-parse", "HEAD"]) {
        Ok(commit) => commit,
        Err(err) => {
            error!("{err:#}");
            return StepVerdict::Abort;
        }
    };

    if let Err(err) = repo.sync_submodules() {
        warn!("Skipping {commit}: {err:#}");
        return StepVerdict::Skip;
    }
    let series = measure(&plan.spec, &commit, &plan.repository);
    // The tree was clean when rafn started, so any tracked change is the
    // benchmark's own (e.g. a rewritten Cargo.lock) and would otherwise ride
    // along into the next candidate's checkout.
    if let Err(err) = repo.discard_tracked_changes() {
        error!("{err:#}");
        return StepVerdict::Abort;
    }
    let series = match series {
        Ok(series) => series,
        Err(err) => {
            warn!("Skipping {commit}: {err:#}");
            return StepVerdict::Skip;
        }
    };

    let report = comparison::compare(&plan.baseline, &series, plan.threshold_pct);
    let Some(verdict) = step_verdict(&report) else {
        warn!("Skipping {commit}: not every regressed benchmark could be measured");
        return StepVerdict::Skip;
    };
    comparison::print_report(&report);

    let state_dir = plan_path.parent().unwrap_or_else(|| Path::new("."));
    if let Err(err) = write_json(&step_path(state_dir, &commit), &series) {
        error!("{err:#}");
        return StepVerdict::Abort;
    }
    verdict
}

/// Any regressed series proves the commit bad, but calling it good needs
/// every baseline series measured: a series missing here may be the one that
/// already regressed, and a wrong "good" discards the half of history that
/// holds the culprit. `None` means no conclusive evidence either way.
fn step_verdict(report: &Report) -> Option<StepVerdict> {
    if report.summary.has_regressions {
        return Some(StepVerdict::Bad);
    }
    let all_measured = report
        .comparisons
        .iter()
        .all(|c| matches!(c.outcome, Outcome::Compared { .. } | Outcome::Added { .. }));
    let any_measured = report
        .comparisons
        .iter()
        .any(|c| matches!(c.outcome, Outcome::Compared { .. }));
    (all_measured && any_measured).then_some(StepVerdict::Good)
}

fn regressed_baseline(good: &[SeriesMean], report: &Report) -> Vec<SeriesMean> {
    good.iter()
        .filter(|series| {
            report.comparisons.iter().any(|c| {
                c.key == series.key
                    && matches!(
                        c.outcome,
                        Outcome::Compared {
                            verdict: Verdict::Regressed,
                            ..
                        }
                    )
            })
        })
        .cloned()
        .collect()
}

/// Build (if asked) and benchmark the currently checked-out commit.
fn measure(
    spec: &MeasureSpec,
    commit: &str,
    repository: &RepositoryRef,
) -> Result<Vec<SeriesMean>> {
    if let Some(build_cmd) = &spec.build_cmd {
        info!("Building: {build_cmd}");
        let status = shell(build_cmd)
            .current_dir(&spec.work_dir)
            .status()
            .with_context(|| format!("Failed to spawn build command `{build_cmd}`"))?;
        ensure!(status.success(), "build command failed ({status})");
    }

    let framework_config =
        framework::detect_framework_from(spec.work_dir.clone(), &spec.bench_args)?;
    let mut runs = Vec::with_capacity(spec.runs.get() as usize);
    for _ in 0..spec.runs.get() {
        clear_stale_results(&framework_config.results_strategy)?;
        framework_config.results_strategy.ensure_dir()?;
        for command in &framework_config.commands {
            info!("Running: {}", command.display());
            let result = runner::run_benchmark(command)?;
            if !result.exit_status.success() {
                runner::replay_stderr_on_failure(&result.stderr);
                bail!("`{}` failed ({})", command.display(), result.exit_status);
            }
        }

        let discovered = discovery::discover_results(&framework_config.results_strategy, None)?;
        let sets = ingest::parse_discovered(
            &discovered,
            repository,
            commit,
            // Branch stays empty so series keys from different commits match
            // exactly; bisect checkouts are detached anyway.
            None,
            &Uuid::new_v4().to_string(),
            timestamp_now(),
        )?;
        let series = filter_series(
            comparison::flatten_series(&sets),
            spec.benchmark_filter.as_deref(),
        );
        ensure!(!series.is_empty(), "no matching benchmark results");
        runs.push(series);
    }
    Ok(average_runs(runs))
}

/// Results left over from another commit would otherwise be read back as
/// this commit's measurement, e.g. a benchmark removed or renamed at this
/// commit whose old Criterion directory still exists.
fn clear_stale_results(strategy: &ResultsStrategy) -> Result<()> {
    let remove = |path: &Path| -> Result<()> {
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else if path.exists() {
            std::fs::remove_file(path)
        } else {
            return Ok(());
        };
        removed.with_context(|| format!("Failed to remove stale results {}", path.display()))
    };
    match strategy {
        ResultsStrategy::JsonFile(path) | ResultsStrategy::CriterionDirectory(path) => remove(path),
        ResultsStrategy::JsonDirectory { dir, .. } => remove(dir),
    }
}

fn filter_series(series: Vec<SeriesMean>, filter: Option<&str>) -> Vec<SeriesMean> {
    match filter {
        None => series,
        Some(filter) => series
            .into_iter()
            .filter(|s| s.key.benchmark_name.contains(filter))
            .collect(),
    }
}

/// Average each series' mean across runs, in first-seen order. A series
/// missing from some runs is averaged over the runs that produced it.
fn average_runs(runs: Vec<Vec<SeriesMean>>) -> Vec<SeriesMean> {
    let mut order: Vec<SeriesMean> = Vec::new();
    let mut sums: HashMap<(comparison::SeriesKey, String), (f64, u32)> = HashMap::new();
    for series in runs.into_iter().flatten() {
        let slot = (series.key.clone(), series.unit.clone());
        match sums.get_mut(&slot) {
            Some((sum, count)) => {
                *sum += series.mean;
                *count += 1;
            }
            None => {
                sums.insert(slot, (series.mean, 1));
                order.push(series);
            }
        }
    }
    order
        .into_iter()
        .map(|mut series| {
            if let Some((sum, count)) = sums.get(&(series.key.clone(), series.unit.clone())) {
                series.mean = sum / f64::from(*count);
            }
            series
        })
        .collect()
}

/// Series parsers require a repository identity, but bisect results never
/// leave the machine, so a repository without a remote still works.
fn unpublished_repository() -> RepositoryRef {
    RepositoryRef {
        forge: "local".to_string(),
        owner: "local".to_string(),
        repository: "local".to_string(),
    }
}

fn reset() -> Result<()> {
    let repo = Repo::discover()?;
    let state_dir = repo.state_dir()?;
    let session_path = state_dir.join(SESSION_FILE);
    if !session_path.exists() {
        ensure!(
            !repo.bisect_in_progress()?,
            "No rafn bisect session found, but a git bisect is in progress; \
             it was not started by rafn, so end it with `git bisect reset`."
        );
        if state_dir.exists() {
            std::fs::remove_dir_all(&state_dir)?;
        }
        info!("Nothing to reset.");
        return Ok(());
    }

    let session: Session = read_json(&session_path)?;
    cleanup(&repo, &state_dir, &session)?;
    info!("Bisect state cleaned up.");
    Ok(())
}

fn cleanup(repo: &Repo, state_dir: &Path, session: &Session) -> Result<()> {
    if repo.bisect_in_progress()? {
        // A stale session file must not license resetting a bisect the user
        // started by hand after rafn's own one ended.
        ensure!(
            repo.bisect_started_as(session.bisect.as_ref())?,
            "A git bisect not started by this rafn session is in progress; leaving it \
             alone. End it with `git bisect reset`, then rerun `rafn bisect --reset`."
        );
        repo.git(&["bisect", "reset"])?;
        repo.sync_submodules()?;
    } else {
        repo.restore(&session.original_head)?;
    }
    repo.restore_submodule_heads(&session.submodule_heads)?;
    if state_dir.exists() {
        std::fs::remove_dir_all(state_dir)
            .with_context(|| format!("Failed to remove {}", state_dir.display()))?;
    }
    Ok(())
}

#[derive(Debug)]
struct CommitInfo {
    sha: String,
    author: String,
    subject: String,
}

// stdout is this CLI's output contract, not debug noise.
#[allow(clippy::print_stdout)]
fn print_culprit(commit: &CommitInfo, report: &Report) {
    println!();
    println!("First bad commit: {}", commit.sha);
    println!("Author: {}", commit.author);
    println!("    {}", commit.subject);
    println!();
    comparison::print_report(report);
}

fn step_path(state_dir: &Path, commit: &str) -> PathBuf {
    state_dir.join(STEPS_DIR).join(format!("{commit}.json"))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(value)?;
    // Write-then-rename, so an interruption never leaves a truncated session
    // that would stop `--reset` from recovering the original checkout.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).with_context(|| format!("Failed to write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("Failed to write {}", path.display()))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let json = std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_slice(&json).with_context(|| format!("Failed to parse {}", path.display()))
}

#[cfg(unix)]
fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
}

#[cfg(windows)]
fn shell(script: &str) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", script]);
    command
}

/// Git operations pinned to one repository root, independent of the cwd.
#[derive(Debug)]
struct Repo {
    root: PathBuf,
}

impl Repo {
    fn discover() -> Result<Self> {
        let root =
            git::detect_git_root().context("`rafn bisect` must run inside a git repository")?;
        Ok(Self { root })
    }

    fn state_dir(&self) -> Result<PathBuf> {
        Ok(self
            .root
            .join(self.git(&["rev-parse", "--git-path", STATE_DIR])?))
    }

    fn submodule(&self, path: &str) -> Repo {
        Repo {
            root: self.root.join(path),
        }
    }

    /// HEADs of all initialized submodules, parents before children.
    fn submodule_heads(&self) -> Result<Vec<SubmoduleHead>> {
        self.git(&[
            "submodule",
            "foreach",
            "--recursive",
            "--quiet",
            r#"echo "$displaypath""#,
        ])?
        .lines()
        .map(|path| {
            Ok(SubmoduleHead {
                path: path.to_string(),
                head: self.submodule(path).head()?,
            })
        })
        .collect()
    }

    fn restore_submodule_heads(&self, heads: &[SubmoduleHead]) -> Result<()> {
        for SubmoduleHead { path, head } in heads {
            if let OriginalHead::Branch(branch) = head {
                self.submodule(path)
                    .git(&["checkout", "--quiet", branch])
                    .with_context(|| format!("Failed to restore submodule {path} to {branch}"))?;
            }
        }
        Ok(())
    }

    fn command(&self) -> Command {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root);
        command
    }

    /// Run git and return its trimmed stdout, failing with git's stderr.
    fn git(&self, args: &[&str]) -> Result<String> {
        let output = self
            .command()
            .args(args)
            .output()
            .with_context(|| format!("Failed to run git {}", args.join(" ")))?;
        ensure!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn is_dirty(&self) -> Result<bool> {
        // Untracked files can't be clobbered by checkouts, and build output
        // like `.rafn/` or `target/` is routinely left untracked.
        Ok(!self
            .git(&["status", "--porcelain", "--untracked-files=no"])?
            .is_empty())
    }

    fn bisect_in_progress(&self) -> Result<bool> {
        let path = self.git(&["rev-parse", "--git-path", "BISECT_START"])?;
        Ok(self.root.join(path).exists())
    }

    /// Whether the in-progress bisect was started with exactly `range`.
    fn bisect_started_as(&self, range: Option<&BisectRange>) -> Result<bool> {
        let Some(range) = range else {
            return Ok(false);
        };
        let log = self.git(&["bisect", "log"])?;
        Ok(log
            .lines()
            .find(|line| line.starts_with("git bisect start"))
            .is_some_and(|line| line.contains(&range.bad) && line.contains(&range.good)))
    }

    /// `git checkout` overwrites ignored files by default, and `git bisect`
    /// gives no way to change that, so refuse up front when an ignored path
    /// exists that some candidate commit tracks. Candidates' trees only
    /// contain paths from `good`'s tree or touched between good and bad.
    fn ignored_collisions(&self, good: &str, bad: &str) -> Result<Vec<String>> {
        let ignored = self.git(&[
            "-c",
            "core.quotepath=off",
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ])?;
        if ignored.is_empty() {
            return Ok(Vec::new());
        }
        let mut tracked: std::collections::BTreeSet<String> = self
            .git(&[
                "-c",
                "core.quotepath=off",
                "ls-tree",
                "-r",
                "--name-only",
                good,
            ])?
            .lines()
            .map(str::to_string)
            .collect();
        tracked.extend(
            self.git(&[
                "-c",
                "core.quotepath=off",
                "log",
                "--format=",
                "--name-only",
                &format!("{good}..{bad}"),
            ])?
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string),
        );
        Ok(ignored
            .lines()
            .filter(|path| match path.strip_suffix('/') {
                // `--directory` collapses wholly-ignored directories.
                Some(_) => tracked.iter().any(|t| t.starts_with(path)),
                None => tracked.contains(*path),
            })
            .map(str::to_string)
            .collect())
    }

    fn discard_tracked_changes(&self) -> Result<()> {
        self.git(&["reset", "--hard", "--quiet"])?;
        self.git(&[
            "submodule",
            "foreach",
            "--recursive",
            "--quiet",
            "git reset --hard --quiet",
        ])
        .map(drop)
    }

    fn resolve_commit(&self, rev: &str) -> Result<String> {
        self.git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .with_context(|| format!("`{rev}` is not a commit"))
    }

    fn head(&self) -> Result<OriginalHead> {
        match self.git(&["symbolic-ref", "--quiet", "--short", "HEAD"]) {
            Ok(branch) => Ok(OriginalHead::Branch(branch)),
            Err(_) => Ok(OriginalHead::Detached(self.git(&["rev-parse", "HEAD"])?)),
        }
    }

    fn checkout_detached(&self, commit: &str) -> Result<()> {
        self.git(&["checkout", "--quiet", "--detach", commit])?;
        self.sync_submodules()
    }

    fn restore(&self, head: &OriginalHead) -> Result<()> {
        match head {
            OriginalHead::Branch(branch) => self.git(&["checkout", "--quiet", branch]),
            OriginalHead::Detached(sha) => self.git(&["checkout", "--quiet", "--detach", sha]),
        }?;
        self.sync_submodules()
    }

    /// Checkouts (ours and `git bisect`'s) move only the superproject's
    /// gitlinks; without this a regression from a submodule bump measures
    /// identically on both sides. Deliberately no `--init`: submodules the
    /// user never initialized stay untouched.
    fn sync_submodules(&self) -> Result<()> {
        self.git(&["submodule", "update", "--recursive", "--quiet"])
            .map(drop)
    }

    fn describe(&self, commit: &str) -> Result<CommitInfo> {
        let out = self.git(&["log", "-1", "--format=%H%x00%an <%ae>%x00%s", commit])?;
        let mut fields = out.splitn(3, '\0').map(str::to_string);
        match (fields.next(), fields.next(), fields.next()) {
            (Some(sha), Some(author), Some(subject)) => Ok(CommitInfo {
                sha,
                author,
                subject,
            }),
            _ => bail!("Unexpected `git log` output for {commit}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparison::SeriesKey;

    fn series(name: &str, unit: &str, mean: f64) -> SeriesMean {
        SeriesMean {
            key: SeriesKey {
                benchmark_name: name.to_string(),
                metric_name: "wall_time".to_string(),
                parameters_json: String::new(),
                branch: String::new(),
            },
            unit: unit.to_string(),
            mean,
        }
    }

    #[test]
    fn average_runs_averages_per_series_in_first_seen_order() {
        let averaged = average_runs(vec![
            vec![series("a", "ns", 10.0), series("b", "ns", 100.0)],
            vec![series("b", "ns", 300.0), series("a", "ns", 30.0)],
        ]);

        let names: Vec<_> = averaged
            .iter()
            .map(|s| s.key.benchmark_name.as_str())
            .collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(averaged[0].mean, 20.0);
        assert_eq!(averaged[1].mean, 200.0);
    }

    #[test]
    fn average_runs_uses_only_runs_that_produced_the_series() {
        let averaged = average_runs(vec![
            vec![series("a", "ns", 10.0)],
            vec![series("a", "ns", 20.0), series("late", "ns", 7.0)],
        ]);

        assert_eq!(averaged.len(), 2);
        assert_eq!(averaged[0].mean, 15.0);
        assert_eq!(averaged[1].mean, 7.0);
    }

    #[test]
    fn average_runs_keeps_different_units_apart() {
        let averaged = average_runs(vec![vec![
            series("a", "ns", 10.0),
            series("a", "bytes", 4.0),
        ]]);
        assert_eq!(averaged.len(), 2);
    }

    #[test]
    fn filter_series_matches_substring_of_benchmark_name() {
        let filtered = filter_series(
            vec![series("parse/json", "ns", 1.0), series("encode", "ns", 1.0)],
            Some("parse"),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].key.benchmark_name, "parse/json");
    }

    #[test]
    fn baseline_keeps_only_regressed_series() {
        let good = vec![series("slow", "ns", 100.0), series("same", "ns", 100.0)];
        let bad = vec![series("slow", "ns", 200.0), series("same", "ns", 100.0)];
        let report = comparison::compare(&good, &bad, 5.0);

        let baseline = regressed_baseline(&good, &report);

        assert_eq!(baseline.len(), 1);
        assert_eq!(baseline[0].key.benchmark_name, "slow");
    }

    #[test]
    fn baseline_is_empty_without_regression() {
        let good = vec![series("a", "ns", 100.0)];
        let faster = vec![series("a", "ns", 50.0)];
        let report = comparison::compare(&good, &faster, 5.0);

        assert!(regressed_baseline(&good, &report).is_empty());
    }

    #[test]
    fn step_verdict_classifies_against_baseline() {
        let baseline = vec![series("a", "ns", 100.0)];

        let within = comparison::compare(&baseline, &[series("a", "ns", 102.0)], 5.0);
        let regressed = comparison::compare(&baseline, &[series("a", "ns", 150.0)], 5.0);

        assert_eq!(step_verdict(&within), Some(StepVerdict::Good));
        assert_eq!(step_verdict(&regressed), Some(StepVerdict::Bad));
    }

    #[test]
    fn step_verdict_is_none_when_no_baseline_series_was_measured() {
        let baseline = vec![series("a", "ns", 100.0)];
        let report = comparison::compare(&baseline, &[series("other", "ns", 1.0)], 5.0);

        assert_eq!(step_verdict(&report), None);
    }

    #[test]
    fn step_verdict_is_none_when_a_baseline_series_is_missing_and_rest_unchanged() {
        let baseline = vec![series("a", "ns", 100.0), series("b", "ns", 100.0)];
        let report = comparison::compare(&baseline, &[series("a", "ns", 100.0)], 5.0);

        assert_eq!(step_verdict(&report), None);
    }

    #[test]
    fn step_verdict_is_bad_when_any_measured_series_regressed() {
        let baseline = vec![series("a", "ns", 100.0), series("b", "ns", 100.0)];
        let report = comparison::compare(&baseline, &[series("a", "ns", 200.0)], 5.0);

        assert_eq!(step_verdict(&report), Some(StepVerdict::Bad));
    }

    #[test]
    fn step_verdict_ignores_series_absent_from_baseline() {
        let baseline = vec![series("a", "ns", 100.0)];
        let report = comparison::compare(
            &baseline,
            &[series("a", "ns", 100.0), series("new", "ns", 1.0)],
            5.0,
        );

        assert_eq!(step_verdict(&report), Some(StepVerdict::Good));
    }

    #[test]
    fn clear_stale_results_removes_result_directories_and_files() -> Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let criterion = tmp.path().join("target/criterion");
        std::fs::create_dir_all(criterion.join("old_bench/new"))?;
        let json = tmp.path().join("out.json");
        std::fs::write(&json, "{}")?;

        clear_stale_results(&ResultsStrategy::CriterionDirectory(criterion.clone()))?;
        clear_stale_results(&ResultsStrategy::JsonFile(json.clone()))?;
        clear_stale_results(&ResultsStrategy::JsonDirectory {
            dir: tmp.path().join("missing"),
            required_suffix: None,
        })?;

        assert!(!criterion.exists());
        assert!(!json.exists());
        Ok(())
    }

    #[test]
    fn write_json_replaces_atomically_without_leftovers() -> Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let path = tmp.path().join("state/session.json");

        write_json(&path, &vec![1])?;
        write_json(&path, &vec![2])?;

        assert_eq!(read_json::<Vec<i32>>(&path)?, [2]);
        assert!(!path.with_extension("json.tmp").exists());
        Ok(())
    }

    #[test]
    fn exit_codes_match_cli_and_git_bisect_contracts() {
        assert_eq!(ExitCode::from(BisectExit::Found), ExitCode::from(0));
        assert_eq!(ExitCode::from(BisectExit::NoRegression), ExitCode::from(1));
        assert_eq!(ExitCode::from(BisectExit::DirtyTree), ExitCode::from(2));
        assert_eq!(ExitCode::from(BisectExit::Failed), ExitCode::from(3));
        assert_eq!(ExitCode::from(StepVerdict::Skip), ExitCode::from(125));
    }
}
