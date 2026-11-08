# Rafn

Lightweight benchmark uploader for Rust Criterion, Java JMH, C# BenchmarkDotNet and C++ Google Benchmark projects.

A benchmark run in CI shows numbers for one commit, not whether a change made the code slower. rafn runs a project's existing benchmarks, saves the results as a snapshot per commit and reports regressions against the previous snapshot. It uploads snapshots to the rafn cloud service for trends and commit-to-commit comparison, and `rafn bisect` finds the commit that introduced a regression.

## Installation

| Method | Command |
|--------|---------|
| Homebrew (macOS/Linux) | `brew install voron-software/tap/rafn` |
| winget (Windows) | `winget install VoronSoftware.Rafn` |
| pip | `pip install rafn` |
| npm | `npm install -g @voron-software/rafn` |
| cargo-binstall | `cargo binstall rafn` |
| cargo install | `cargo install rafn` |

## Quickstart

From the directory of a project that uses one of the supported frameworks, keep snapshots on disk with the local backend:

```sh
printf '[backend]\ntype = "local"\n' > rafn.toml
rafn bench        # runs the benchmarks and saves a snapshot for the current commit
# change the code, commit, then
rafn bench        # compares against the previous commit's snapshot and reports regressions
```

To upload snapshots to the rafn cloud service for trends and commit-to-commit comparison, switch to the cloud backend and provide a credential as described in the [authentication guide](https://github.com/voron-software/rafn/blob/main/docs/guides/authentication.md).

## Documentation

Guides for authentication, the GitHub Action, bisecting and the framework examples, and the architecture docs, are in the [documentation index](https://github.com/voron-software/rafn/blob/main/docs/README.md).

## Contributing and license

See the [contributing guide](https://github.com/voron-software/.github/blob/main/.github/CONTRIBUTING.md). rafn is licensed under [Apache-2.0](https://github.com/voron-software/rafn/blob/main/LICENSE).
