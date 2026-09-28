# Rafn

Lightweight benchmark uploader

## Installation

| Method | Command |
|--------|---------|
| Homebrew (macOS/Linux) | `brew install voron-software/tap/rafn` |
| winget (Windows) | `winget install VoronSoftware.Rafn` |
| pip | `pip install rafn` |
| npm | `npm install -g @voron-software/rafn` |
| cargo-binstall | `cargo binstall rafn` |
| cargo install | `cargo install rafn` |

## GitHub Actions

Use this repository as a GitHub Action to install the `rafn` CLI and run it in a workflow. By default it runs `rafn bench`:

```yaml
steps:
  - uses: voron-software/rafn@v0.1.0
    with:
      version: 0.1.0
      command: bench
      args: --no-fail
```

Installation is handled by [`taiki-e/install-action`](https://github.com/taiki-e/install-action), falling back to `cargo-binstall`. Set `version: latest` (the default) to install the newest release, or pin a crate version such as `0.1.0`.

`command` selects the rafn subcommand to invoke (`bench`, `push`, `trend`, `compare`, `bisect`, or `config`), and `args` is a space-separated string of additional arguments passed through to that subcommand. Set `working-directory` to run the command from a benchmark project nested within a monorepo, such as `crates/foo` or `benchmarks/app`.

## Bisecting regressions

`rafn bisect` finds the commit that introduced a performance regression. It benchmarks `--good` and `--bad` to confirm a regression exists, then drives `git bisect run`, classifying each candidate commit against the good baseline:

```sh
rafn bisect --good v1.2.0 --bad main --benchmark parse --runs 3
```

Only benchmarks that regressed between `--good` and `--bad` decide each step. Commits that fail to build or benchmark, or that don't produce every one of those benchmarks, are skipped. Initialized submodules follow each checkout. `--build-cmd` runs a shell command before benchmarking each commit, and arguments after `--` go to the benchmark framework, as with `rafn bench`. The working tree must have no uncommitted changes to tracked files. Tracked files the benchmark itself modifies are reset after each commit, and rafn refuses to start if a checkout in the range would overwrite an ignored file.

Exit codes: `0` first bad commit found, `1` no regression between good and bad, `2` dirty working tree, `3` bisect failed. If a run is interrupted, `rafn bisect --reset` restores the original checkout and removes the temporary state in `.rafn/bisect/`.
