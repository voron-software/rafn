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

From the directory of a project that uses one of the supported frameworks:

```sh
rafn init                         # writes rafn.toml for the cloud backend
rafn bench                        # runs the benchmarks, saves a snapshot, reports regressions
export RAFN_API_KEY=<your key>
rafn push                         # uploads the snapshot for the current commit
```

In GitHub Actions no key is needed; see [Authentication](https://github.com/voron-software/rafn/blob/main/docs/guides/authentication.md).

## Documentation

[docs/README.md](https://github.com/voron-software/rafn/blob/main/docs/README.md) indexes the guides and architecture docs.

## Contributing and license

See the [contributing guide](https://github.com/voron-software/.github/blob/main/CONTRIBUTING.md). rafn is licensed under [Apache-2.0](https://github.com/voron-software/rafn/blob/main/LICENSE).
