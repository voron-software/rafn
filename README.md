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

## Authentication

`rafn push`, `rafn trend` and `rafn compare` talk to the rafn cloud service, which requires a credential. rafn picks one in this order:

1. The API key in `RAFN_API_KEY`, if set. An explicit key always wins, even in GitHub Actions.
2. In GitHub Actions, the workflow's OIDC token, if the job has `permissions: id-token: write`. No secret is involved.
3. Otherwise rafn stops with an error that says how to provide one.

`rafn bench` and `rafn bisect` run entirely locally, and with `[backend] type = "local"` nothing needs a credential. `rafn push --dry-run` does not need one either, because it never contacts the service.

### GitHub Actions

Grant the job `id-token: write` and rafn authenticates with the OIDC token GitHub issues for the run. There is no secret to create or rotate:

```yaml
jobs:
  push:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
    steps:
      - uses: actions/checkout@v4
      - uses: voron-software/rafn@v0.1.7
        with:
          command: push
```

rafn requests the token from the runner once per command, with the audience `rafn`, and sends it with every request. Keyless authentication needs a rafn release newer than 0.1.7, which the action's default `version: latest` installs. If the job lacks the permission, rafn says so.

To authenticate with an API key instead (for example to use a specific key rather than the repository's identity), store it as a secret and pass it with the `api-key` input (available from v0.1.7; earlier versions ignore it), which sets `RAFN_API_KEY` for the command:

```yaml
steps:
  - uses: voron-software/rafn@v0.1.7
    with:
      command: push
      api-key: ${{ secrets.RAFN_API_KEY }}
```

GitHub grants neither secrets nor `id-token: write` to workflows triggered by pull requests from forks, so `push`, `trend` and `compare` fail there with a message saying no credential is available.

### API key

Outside GitHub Actions, for example in other CI systems or on a workstation, provide a key through the `RAFN_API_KEY` environment variable:

```sh
export RAFN_API_KEY=<your key>
rafn push
```

The key is read only from the environment, never from `rafn.toml` or `~/.config/rafn/config.toml`, so it cannot be committed by accident. It is sent as a bearer token with every request, as is the OIDC token, so the API URL must be `https://`. Plain `http://` is accepted only for loopback hosts (`localhost`, `127.0.0.1`, `[::1]`) such as a local development server; rafn refuses any other `http://` URL rather than send a credential unencrypted.

If the service rejects the credential, rafn says so. If the credential is valid but the service refuses the request (for example because the subscription is inactive or the quota is used up), rafn prints the reason the service gave. If something in front of the service (a proxy, a firewall, an access policy) refuses the request before it reaches rafn, the error shows the HTTP status instead, and says nothing about the credential or the subscription.

## Bisecting regressions

`rafn bisect` finds the commit that introduced a performance regression. It benchmarks `--good` and `--bad` to confirm a regression exists, then drives `git bisect run`, classifying each candidate commit against the good baseline:

```sh
rafn bisect --good v1.2.0 --bad main --benchmark parse --runs 3
```

Only benchmarks that regressed between `--good` and `--bad` decide each step. Commits that fail to build or benchmark, or that don't produce every one of those benchmarks, are skipped. `--build-cmd` runs a shell command before benchmarking each commit, and arguments after `--` go to the benchmark framework, as with `rafn bench`. The working tree must have no uncommitted changes to tracked files.

Exit codes: `0` first bad commit found, `1` no regression between good and bad, `2` dirty working tree, `3` bisect failed. If a run is interrupted, `rafn bisect --reset` restores the original checkout and removes rafn's temporary state (kept under `.git/rafn/bisect/`).
