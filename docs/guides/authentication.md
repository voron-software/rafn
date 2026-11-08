# Authentication

`rafn push`, `rafn trend` and `rafn compare` talk to the rafn cloud service, which requires a credential. rafn picks one in this order:

1. The API key in `RAFN_API_KEY`, if set. An explicit key always wins, even in GitHub Actions.
2. In GitHub Actions, the workflow's OIDC token, if the job has `permissions: id-token: write`. No secret is involved.
3. Otherwise rafn stops with an error that says how to provide one.

`rafn bench` and `rafn bisect` run entirely locally, and with `[backend] type = "local"` nothing needs a credential. `rafn push --dry-run` does not need one either, because it never contacts the service.

[Credential handling](../architecture/credential-handling.md) describes how this works inside rafn.

## GitHub Actions

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

rafn requests the token from the runner once per command, with the audience `rafn`, and sends it with every request. Keyless authentication needs rafn 0.1.7 or newer, which the action's default `version: latest` installs. If the job lacks the permission, rafn says so.

To authenticate with an API key instead (for example to use a specific key rather than the repository's identity), store it as a secret and pass it with the `api-key` input, which sets `RAFN_API_KEY` for the command:

```yaml
steps:
  - uses: voron-software/rafn@v0.1.7
    with:
      command: push
      api-key: ${{ secrets.RAFN_API_KEY }}
```

GitHub grants neither secrets nor `id-token: write` to workflows triggered by pull requests from forks, so `push`, `trend` and `compare` fail there with a message saying no credential is available.

## API key

Outside GitHub Actions, for example in other CI systems or on a workstation, provide a key through the `RAFN_API_KEY` environment variable:

```sh
export RAFN_API_KEY=<your key>
rafn push
```

The key is read only from the environment, never from `rafn.toml` or `~/.config/rafn/config.toml`, so it cannot be committed by accident.

## Plain HTTP

The API key and the OIDC token are sent as bearer tokens with every request, so the API URL must be `https://`. Plain `http://` is accepted only for loopback hosts (`localhost`, `127.0.0.0/8`, `[::1]`) such as a local development server. rafn refuses any other `http://` URL rather than send a credential unencrypted.

## When the service refuses a request

- If the service rejects the credential, rafn says so and names the credential it used.
- If the credential is valid but the service refuses the request, for example because the subscription is inactive or the quota is used up, rafn prints the reason the service gave.
- If something in front of the service, such as a proxy, a firewall or an access policy, refuses the request before it reaches rafn, the error shows the HTTP status instead. It says nothing about the credential or the subscription.
