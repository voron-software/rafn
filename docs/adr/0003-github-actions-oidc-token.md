# 0003: GitHub Actions OIDC token as a keyless credential

**Status:** Accepted

## Context

[0002](0002-api-key-from-env-via-channel-interceptor.md) authenticates with an
API key from `RAFN_API_KEY`. In GitHub Actions that means every user creates a
key, stores it as a secret and rotates it. A job with `permissions: id-token:
write` can instead ask the runner for a short-lived JWT that proves which
repository and workflow run is calling, and the cloud backend accepts that JWT
as the bearer credential when it is issued for the audience `rafn`.

## Decision

- Resolution order lives in one place, `src/config/credential.rs`:
  `RAFN_API_KEY` if set (an explicit key always wins, even in Actions, and a
  set-but-broken key is an error, not a reason to fall through); else, when the
  runner provides `ACTIONS_ID_TOKEN_REQUEST_URL` and
  `ACTIONS_ID_TOKEN_REQUEST_TOKEN`, an OIDC token; else an error. When
  `GITHUB_ACTIONS=true` but the two variables are absent, the error says the
  job lacks `id-token: write` rather than giving the generic message.
- The work is split so that what can fail locally runs first and without I/O.
  `CredentialSource` is the decision plus validation (including the request
  URL and request token). Its `resolve` makes the one network request and
  returns a `Credential`, the enum the remote store holds, so the store cannot
  carry an unfetched token. `RemoteBackend` construction validates the source
  and the API URL first and resolves last, so a bad API URL never costs a token
  request.
- The token is fetched once per invocation, when the backend is built: it lives
  for minutes and the CLI for seconds. The request is `GET <url>&audience=rafn`
  with `authorization: bearer <request token>`, expects `{"value": "<jwt>"}`,
  and has a 10 second deadline. The value must be three non-empty base64url
  segments; the signature and claims are the backend's to check.
- Both secrets get the `ApiKey` treatment: redacting `Debug`, no `Display`,
  sensitive header values, and error messages that never echo the request
  token, the JWT or any part of the response body (a malformed response may be
  the secret).
- `http://` for the token URL follows the API URL rule from 0001: loopback
  hosts only. Redirects are not followed, so the request token cannot be
  replayed to another host, and the body is capped at 64 KiB.
- Rejection messages name the credential in use. An `UNAUTHENTICATED` answer to
  an OIDC token says the token was rejected and offers `RAFN_API_KEY` as the
  alternative, instead of sending a keyless user to look for a key they never
  set.
- The request is made with `hyper` and `tokio-rustls` directly
  (`src/config/http_get.rs`), using the same provider and OS trust store as the
  gRPC channel. All of those crates are already in the build through tonic, so
  the lockfile gains no package. tonic's channel cannot make this request: it
  speaks only HTTP/2, which the token endpoint need not offer. `ureq` or
  `reqwest` would add a dependency tree and, with their default root stores, a
  second source of trust roots and the `CDLA-Permissive-2.0` license
  (`webpki-roots`) to `deny.toml`, to save roughly 150 lines.
- `rafn push --dry-run` and `[backend] type = "local"` never build a remote
  backend, so they resolve no credential and make no token request.

## Consequences

- The request goes straight to the host in the URL: no `HTTPS_PROXY` support.
  The gRPC channel has none either, so a proxied runner already could not reach
  the service; add both together if that comes up.
- A host with no CA bundle fails the token request with a message naming
  `ca-certificates`, as it already would for the API.
- Pull requests from forks get neither secrets nor `id-token: write`, so they
  still cannot reach the cloud backend; the error says so.
- The CLI cannot tell a token the backend will accept from one it will not
  (wrong repository owner, unregistered account); that surfaces as the
  backend's `UNAUTHENTICATED` message.
- The audience `rafn` is a contract with the backend. Changing it is a
  coordinated change.
