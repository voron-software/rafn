# Documentation

The [project README](../README.md) covers installation and a quickstart.

## Architecture

How rafn is built and why.

- [Transport security](architecture/transport-security.md): TLS provider and trust roots, and which URLs may carry a credential.
- [Credential handling](architecture/credential-handling.md): credential resolution order, the channel interceptor, the OIDC token fetch and error mapping.

## Guides

Task-oriented how-tos.

- [Authentication](guides/authentication.md): API keys, keyless GitHub Actions OIDC, and what refusal messages mean.
- [GitHub Actions](guides/github-actions.md): the composite action's inputs and output.
- [Bisecting regressions](guides/bisecting-regressions.md): finding the commit that made a benchmark slower, and the exit codes.
- [Framework examples](guides/framework-examples.md): the example project per framework, run locally or in Docker.
