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
