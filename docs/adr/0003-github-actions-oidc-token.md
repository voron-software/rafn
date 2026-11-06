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
- The request is made with `reqwest` (`src/config/actions_token.rs`) with
  default features off: HTTP/1.1 only, no compression, cookies or system proxy.
  TLS is its `rustls-no-provider` backend, so aws-lc-rs stays out of the build
  as ADR 0001 requires, and the ring provider the gRPC channel uses is
  installed process-wide just before the client is built. Roots come from the
  OS trust store through `rustls-platform-verifier`.
- The client is configured for a credential fetch rather than general use:
  redirects are refused, the 10 second timeout covers connecting and reading
  the body, and the body is read in chunks and abandoned past 64 KiB because
  reqwest has no cap of its own. A non-success status is reported before the
  body is read, and neither the body nor the request URL (reqwest prints it
  by default) reaches an error message.
- tonic's channel could not make this request: it speaks only HTTP/2, which
  the token endpoint need not offer. A hand-written GET on `hyper` and
  `tokio-rustls` would add no packages but would put TLS setup and HTTP edge
  cases in this repository; it was tried and replaced.
- `rafn push --dry-run` and `[backend] type = "local"` never build a remote
  backend, so they resolve no credential and make no token request.

## Consequences

- The lockfile gains `reqwest`, `hyper-rustls`, `rustls-platform-verifier` and
  a handful of crates behind them, most of them Android or wasm32 only and
  never compiled for a release target. `deny.toml` allows
  `CDLA-Permissive-2.0` for one of those (`webpki-root-certs`).
- The OS trust store is read through two code paths: tonic's
  `rustls-native-certs` for the API, `rustls-platform-verifier` for the token
  (the same files on Linux, the platform verifier APIs on macOS and Windows).
  A CA the OS trusts is trusted by both.
- No `HTTPS_PROXY` support: reqwest's `system-proxy` feature is off. The gRPC
  channel has none either, so a proxied runner already could not reach the
  service; turn both on together if that comes up.
- A host with no CA bundle fails the token request at certificate verification
  (unknown issuer); the API connection fails there as well.
- Pull requests from forks get neither secrets nor `id-token: write`, so they
  still cannot reach the cloud backend; the error says so.
- The CLI cannot tell a token the backend will accept from one it will not
  (wrong repository owner, unregistered account); that surfaces as the
  backend's `UNAUTHENTICATED` message.
- The audience `rafn` is a contract with the backend. Changing it is a
  coordinated change.
