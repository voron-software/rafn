//! End-to-end `rafn bisect` runs against a throwaway git repository.
//!
//! The repository poses as a Google Benchmark project: `cmake` on PATH is a
//! no-op stub and the committed `build/bench` script reports the time stored
//! in the committed `perf` file, so each commit's "performance" is data.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, ensure};
use tempfile::TempDir;

const BENCH_SCRIPT: &str = r#"#!/bin/sh
[ -f broken ] && { echo "does not compile" >&2; exit 1; }
for arg in "$@"; do
  case "$arg" in --benchmark_out=*) out="${arg#--benchmark_out=}" ;; esac
done
# Like `cargo bench` refreshing a stale Cargo.lock: the benchmark itself
# dirties a tracked file.
echo touched >> CMakeLists.txt
if [ -f lib/perf ]; then t=$(cat lib/perf); else t=$(cat perf); fi
cat > "$out" <<JSON
{"context":{},"benchmarks":[{"name":"BM_Work","run_type":"iteration","iterations":1000,"real_time":$t,"cpu_time":$t,"time_unit":"ns"}]}
JSON
"#;

struct Fixture {
    repo: TempDir,
    stub_bin: TempDir,
    commits: Vec<String>,
}

impl Fixture {
    /// One commit per entry: `Some(ns)` sets the benchmark time, `None`
    /// commits a build that fails.
    fn new(history: &[Option<u32>]) -> Result<Self> {
        let repo = TempDir::new()?;
        let stub_bin = TempDir::new()?;
        write_executable(&stub_bin.path().join("cmake"), "#!/bin/sh\nexit 0\n")?;

        let fixture = Self {
            repo,
            stub_bin,
            commits: Vec::new(),
        };
        fixture.git(&["init", "--quiet", "--initial-branch=main"])?;
        fixture.git(&["config", "user.name", "Test Author"])?;
        fixture.git(&["config", "user.email", "author@example.com"])?;
        std::fs::write(
            fixture.path().join("CMakeLists.txt"),
            "add_executable(bench bench.cpp)\ntarget_link_libraries(bench benchmark::benchmark)\n",
        )?;
        std::fs::write(fixture.path().join(".gitignore"), ".rafn/\n")?;
        write_executable(&fixture.path().join("build/bench"), BENCH_SCRIPT)?;

        let mut fixture = fixture;
        for (i, entry) in history.iter().enumerate() {
            let broken = fixture.path().join("broken");
            match entry {
                Some(ns) => {
                    std::fs::write(fixture.path().join("perf"), ns.to_string())?;
                    if broken.exists() {
                        std::fs::remove_file(&broken)?;
                    }
                }
                None => std::fs::write(&broken, "")?,
            }
            fixture.git(&["add", "--all"])?;
            fixture.git(&[
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                &format!("commit {i}"),
            ])?;
            fixture.commits.push(fixture.git(&["rev-parse", "HEAD"])?);
        }
        Ok(fixture)
    }

    fn commit(&mut self, message: &str) -> Result<()> {
        self.git(&["add", "--all"])?;
        self.git(&["commit", "--quiet", "--allow-empty", "-m", message])?;
        self.commits.push(self.git(&["rev-parse", "HEAD"])?);
        Ok(())
    }

    /// The session file an interrupted `rafn bisect` leaves behind, with the
    /// (bad, good) commit indices of the bisect it started, if any.
    fn write_session(&self, bisect: Option<(usize, usize)>) -> Result<()> {
        let bisect = match bisect {
            Some((bad, good)) => format!(
                r#"{{"bad":"{}","good":"{}"}}"#,
                self.commits[bad], self.commits[good]
            ),
            None => "null".to_string(),
        };
        let state = self.path().join(".rafn/bisect");
        std::fs::create_dir_all(state.join("steps"))?;
        std::fs::write(
            state.join("session.json"),
            format!(r#"{{"original_head":{{"Branch":"main"}},"bisect":{bisect}}}"#),
        )?;
        Ok(())
    }

    fn path(&self) -> &Path {
        self.repo.path()
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let output = isolated_git_env(Command::new("git").args(args))
            .current_dir(self.path())
            .output()?;
        ensure!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn rafn(&self, args: &[&str]) -> Result<Output> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.stub_bin.path().to_path_buf()];
        paths.extend(std::env::split_paths(&path));
        Ok(
            isolated_git_env(Command::new(env!("CARGO_BIN_EXE_rafn")).args(args))
                .current_dir(self.path())
                .env("PATH", std::env::join_paths(paths)?)
                // Keep the developer's own rafn config from leaking in.
                .env("HOME", self.path())
                .env("XDG_CONFIG_HOME", self.path().join(".config"))
                .output()?,
        )
    }

    fn bisect(&self, good: usize, bad: usize) -> Result<Output> {
        self.rafn(&[
            "bisect",
            "--good",
            &self.commits[good],
            "--bad",
            &self.commits[bad],
            "--threshold",
            "10",
        ])
    }

    fn assert_restored(&self) -> Result<()> {
        assert_eq!(self.git(&["symbolic-ref", "--short", "HEAD"])?, "main");
        let bisect_start = self.git(&["rev-parse", "--git-path", "BISECT_START"])?;
        assert!(
            !self.path().join(bisect_start).exists(),
            "git bisect still in progress"
        );
        assert!(
            !self.path().join(".rafn/bisect").exists(),
            "bisect state left behind"
        );
        assert_eq!(
            self.git(&["status", "--porcelain", "--untracked-files=no"])?,
            "",
            "tracked files left modified"
        );
        Ok(())
    }
}

/// Machine-wide git config and identity (commit signing, hooks, author) must not
/// change how the fixture repository behaves.
fn isolated_git_env(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
}

fn write_executable(path: &PathBuf, content: &str) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("path has a parent")?)?;
    std::fs::write(path, content)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn describe(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn finds_first_bad_commit_and_skips_broken_builds() -> Result<()> {
    // git bisect first tests commit 3 of this history, so the broken build
    // is guaranteed to be visited and must be skipped, not classified.
    let fixture = Fixture::new(&[
        Some(100),
        Some(100),
        Some(100),
        None,
        Some(100),
        Some(200),
        Some(200),
        Some(200),
    ])?;

    let output = fixture.bisect(0, 7)?;

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("First bad commit: {}", fixture.commits[5])),
        "{}",
        describe(&output)
    );
    assert!(
        stdout.contains("Test Author <author@example.com>"),
        "{}",
        describe(&output)
    );
    assert!(stdout.contains("commit 5"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("Skipping {}", fixture.commits[3])),
        "{}",
        describe(&output)
    );
    fixture.assert_restored()
}

#[test]
fn exits_1_when_there_is_no_regression() -> Result<()> {
    let fixture = Fixture::new(&[Some(100), Some(102), Some(101)])?;

    let output = fixture.bisect(0, 2)?;

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr).contains("nothing to bisect"));
    fixture.assert_restored()
}

#[test]
fn exits_2_on_dirty_tree_without_checking_anything_out() -> Result<()> {
    let fixture = Fixture::new(&[Some(100), Some(200)])?;
    std::fs::write(fixture.path().join("perf"), "999")?;

    let output = fixture.bisect(0, 1)?;

    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
    assert_eq!(fixture.git(&["rev-parse", "HEAD"])?, fixture.commits[1]);
    assert_eq!(std::fs::read_to_string(fixture.path().join("perf"))?, "999");
    Ok(())
}

#[test]
fn reset_cleans_up_an_interrupted_session() -> Result<()> {
    let fixture = Fixture::new(&[Some(100), Some(100), Some(200), Some(200)])?;
    // State an interrupted `rafn bisect` leaves behind mid-search.
    fixture.write_session(Some((3, 0)))?;
    fixture.git(&["bisect", "start", &fixture.commits[3], &fixture.commits[0]])?;

    let output = fixture.rafn(&["bisect", "--reset"])?;

    assert!(output.status.success(), "{}", describe(&output));
    fixture.assert_restored()
}

#[test]
fn reset_restores_checkout_after_interruption_during_baseline() -> Result<()> {
    let fixture = Fixture::new(&[Some(100), Some(200)])?;
    fixture.write_session(None)?;
    fixture.git(&["checkout", "--quiet", "--detach", &fixture.commits[0]])?;

    let output = fixture.rafn(&["bisect", "--reset"])?;

    assert!(output.status.success(), "{}", describe(&output));
    fixture.assert_restored()
}

#[test]
fn reset_leaves_a_manual_bisect_alone_despite_stale_session() -> Result<()> {
    let fixture = Fixture::new(&[Some(100), Some(100), Some(200)])?;
    // rafn's own bisect covered 2..0; the user later started 2..1 by hand.
    fixture.write_session(Some((2, 0)))?;
    fixture.git(&["bisect", "start", &fixture.commits[2], &fixture.commits[1]])?;

    let output = fixture.rafn(&["bisect", "--reset"])?;

    assert_eq!(output.status.code(), Some(3), "{}", describe(&output));
    let bisect_start = fixture.git(&["rev-parse", "--git-path", "BISECT_START"])?;
    assert!(fixture.path().join(bisect_start).exists());
    Ok(())
}

#[test]
fn refuses_to_start_over_an_interrupted_session() -> Result<()> {
    let fixture = Fixture::new(&[Some(100), Some(200)])?;
    fixture.write_session(None)?;

    let output = fixture.bisect(0, 1)?;

    assert_eq!(output.status.code(), Some(3), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("rafn bisect --reset"),
        "{}",
        describe(&output)
    );
    assert!(fixture.path().join(".rafn/bisect/session.json").exists());
    Ok(())
}

#[test]
fn refuses_when_a_checkout_would_overwrite_an_ignored_file() -> Result<()> {
    let mut fixture = Fixture::new(&[Some(100)])?;
    std::fs::write(fixture.path().join("generated.txt"), "committed")?;
    fixture.commit("track generated.txt")?;
    fixture.git(&["rm", "--quiet", "--cached", "generated.txt"])?;
    std::fs::write(fixture.path().join(".gitignore"), ".rafn/\ngenerated.txt\n")?;
    std::fs::write(fixture.path().join("perf"), "200")?;
    fixture.commit("stop tracking generated.txt")?;
    std::fs::write(fixture.path().join("generated.txt"), "precious")?;

    let output = fixture.bisect(1, 2)?;

    assert_eq!(output.status.code(), Some(3), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("generated.txt"),
        "{}",
        describe(&output)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("generated.txt"))?,
        "precious"
    );
    fixture.assert_restored()
}

#[test]
fn reset_leaves_a_foreign_git_bisect_alone() -> Result<()> {
    let fixture = Fixture::new(&[Some(100), Some(100), Some(200)])?;
    fixture.git(&["bisect", "start", &fixture.commits[2], &fixture.commits[0]])?;

    let output = fixture.rafn(&["bisect", "--reset"])?;

    assert_eq!(output.status.code(), Some(3), "{}", describe(&output));
    let bisect_start = fixture.git(&["rev-parse", "--git-path", "BISECT_START"])?;
    assert!(fixture.path().join(bisect_start).exists());
    Ok(())
}

#[test]
fn finds_regression_introduced_by_a_submodule_bump() -> Result<()> {
    let upstream = TempDir::new()?;
    let upstream_git = |args: &[&str]| -> Result<String> {
        let output = isolated_git_env(Command::new("git").args(args))
            .current_dir(upstream.path())
            .output()?;
        ensure!(output.status.success(), "git {args:?} failed");
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    upstream_git(&["init", "--quiet", "--initial-branch=main"])?;
    upstream_git(&["config", "user.name", "Test Author"])?;
    upstream_git(&["config", "user.email", "author@example.com"])?;
    std::fs::write(upstream.path().join("perf"), "100")?;
    upstream_git(&["add", "perf"])?;
    upstream_git(&["commit", "--quiet", "-m", "fast"])?;
    std::fs::write(upstream.path().join("perf"), "200")?;
    upstream_git(&["commit", "--quiet", "-am", "slow"])?;
    let slow = upstream_git(&["rev-parse", "HEAD"])?;
    let fast = upstream_git(&["rev-parse", "HEAD~1"])?;

    let mut fixture = Fixture::new(&[Some(100)])?;
    let upstream_path = upstream.path().to_string_lossy().to_string();
    fixture.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "--quiet",
        &upstream_path,
        "lib",
    ])?;
    let lib_dir = fixture.path().join("lib");
    let lib_git = |args: &[&str]| -> Result<()> {
        let output = isolated_git_env(Command::new("git").args(args))
            .current_dir(&lib_dir)
            .output()?;
        ensure!(output.status.success(), "git {args:?} failed");
        Ok(())
    };
    lib_git(&["checkout", "--quiet", &fast])?;
    fixture.commit("add lib")?;
    fixture.commit("unrelated")?;
    lib_git(&["checkout", "--quiet", &slow])?;
    fixture.commit("bump lib")?;
    fixture.commit("unrelated again")?;

    let output = fixture.bisect(1, 4)?;

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("First bad commit: {}", fixture.commits[3])),
        "{}",
        describe(&output)
    );
    fixture.assert_restored()?;
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("lib/perf"))?,
        "200"
    );
    Ok(())
}
