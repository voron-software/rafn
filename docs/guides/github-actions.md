# GitHub Actions

This repository is a composite GitHub Action that installs the `rafn` CLI and runs one rafn command in a workflow. By default it runs `rafn bench`:

```yaml
steps:
  - uses: voron-software/rafn@v0.1.0
    with:
      version: 0.1.0
      command: bench
      args: --no-fail
```

## Inputs

| Input | Default | Meaning |
| --- | --- | --- |
| `version` | `latest` | Crate version to install, such as `0.1.0`. `latest` installs the newest release. |
| `command` | `bench` | The rafn subcommand to run: `init`, `bench`, `push`, `trend`, `compare`, `bisect` or `config`. |
| `args` | empty | Additional arguments for the subcommand, split on whitespace. Quotes are passed through literally, not interpreted. |
| `working-directory` | the workspace root | Directory to run the command in, such as `crates/foo` or `benchmarks/app` in a monorepo. |
| `api-key` | empty | An API key, exported as `RAFN_API_KEY` only when non-empty. See [Authentication](authentication.md). |

The `path` output is the path of the installed `rafn` executable.

## Installation

The action installs rafn with the `taiki-e/install-action` action, which falls back to `cargo-binstall`.

## Pushing results

`push`, `trend` and `compare` need a credential when the backend is the cloud service. Grant the job `id-token: write` or pass `api-key`; [Authentication](authentication.md) has both workflows.
