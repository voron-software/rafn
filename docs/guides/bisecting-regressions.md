# Bisecting regressions

`rafn bisect` finds the commit that introduced a performance regression. It benchmarks `--good` and `--bad` to confirm a regression exists, then drives `git bisect run`, classifying each candidate commit against the good baseline:

```sh
rafn bisect --good v1.2.0 --bad main --benchmark parse --runs 3
```

- Only benchmarks that regressed between `--good` and `--bad` decide each step.
- Commits that fail to build or benchmark, or that do not produce every one of those benchmarks, are skipped.
- `--build-cmd` runs a shell command before benchmarking each commit.
- Arguments after `--` go to the benchmark framework, as with `rafn bench`.
- The working tree must have no uncommitted changes to tracked files.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | First bad commit found |
| `1` | No regression between good and bad |
| `2` | Dirty working tree |
| `3` | Bisect failed |

## Interrupted runs

If a run is interrupted, `rafn bisect --reset` restores the original checkout and removes rafn's temporary state, kept under `.git/rafn/bisect/`.
