# Rafn

Lightweight benchmark uploader

## Installation

| Method | Command |
|--------|---------|
| Homebrew (macOS/Linux) | `brew install voron-software/tap/rafn` |
| winget (Windows) | `winget install VoronSoftware.Rafn` |
| cargo-binstall | `cargo binstall rafn` |
| npm | `npm install -g @voron-software/rafn` |

## GitHub Actions

Use this repository as a GitHub Action to install the `rafn` CLI and run it in a workflow. By default it runs `rafn bench`:

```yaml
steps:
  - uses: voron-software/rafn@v0.1.0
    with:
      version: v0.1.0
      command: bench
      args: --no-fail
```

Set `version: latest` to download the newest GitHub Release. Release assets are expected to use the same names as the npm package binaries: `rafn-linux-x64`, `rafn-linux-arm64`, `rafn-darwin-x64`, `rafn-darwin-arm64`, and `rafn-win32-x64.exe`.

`command` selects the rafn subcommand to invoke (`bench`, `push`, `trend`, `compare`, `bisect`, or `config`), and `args` is a space-separated string of additional arguments passed through to that subcommand. Set `working-directory` to run the command from a benchmark project nested within a monorepo, such as `crates/foo` or `benchmarks/app`.
