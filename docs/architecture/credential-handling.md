# Credential handling

`rafn push`, `rafn trend` and `rafn compare` authenticate to the rafn cloud service with a bearer credential: an API key, or inside GitHub Actions the job's OIDC token. This doc covers how rafn picks the credential, attaches it to RPCs and reports refusals. [Authentication](../guides/authentication.md) is the user-facing guide.

## Resolution order

`CredentialSource::from_env` in `src/config/credential.rs` is the one place that decides:

1. `RAFN_API_KEY` set: an API key, even inside GitHub Actions. An explicit key always wins. A set but broken key (empty, not visible ASCII, or not Unicode) is an error, not a reason to fall through, because silently switching credential would hide a miswired secret.
2. Else, when `ACTIONS_ID_TOKEN_REQUEST_URL` and `ACTIONS_ID_TOKEN_REQUEST_TOKEN` are both set and non-blank: a GitHub Actions OIDC token. The runner sets them only when the job has `permissions: id-token: write`.
3. Else, when `GITHUB_ACTIONS=true`: an error saying the job lacks `id-token: write` and that pull requests from forks never get it.
4. Else an error saying how to set `RAFN_API_KEY`, or how to switch to the local backend.

`[backend] type = "local"` and `rafn push --dry-run` never build the remote backend, so they resolve no credential and make no token request.

## Construction order

What can fail locally runs first and without I/O. `CredentialSource` is the decision plus validation, including the token request URL and request token. Its `resolve` makes the one network request, if the source needs one, and returns a `Credential`. That enum is what the remote store holds, so the store cannot carry an unfetched token.

`RemoteBackend::new` in `src/store/remote.rs` takes the validated source, then parses the API URL, then resolves. A missing key or a bad URL fails before any connection to the service, and a bad URL never costs a token request.

## Keeping secrets out of output

- `ApiKey` and `ActionsToken` are newtypes validated at construction. Each has a redacting `Debug` and no `Display`, so the secret cannot leak through `{:?}` of an enclosing struct or through an error chain.
- Every header value carrying a secret is marked sensitive.
- Error messages never echo the API key, the request token, the JWT or any part of the token response body, because a malformed response may be the secret.
- The key is read only from `RAFN_API_KEY` (`src/config/api_key.rs`), never from `rafn.toml` or `~/.config/rafn/config.toml`. Tests in both config loaders pin that the `RAFN_` environment scan does not pull it into either config type.

## Attaching the credential

One `AuthInterceptor` (`src/store/remote/auth.rs`) is installed on the channel, so no RPC can forget the credential. It sets two headers on every call, overwriting any caller-supplied value:

| Header | Value |
| --- | --- |
| `authorization` | `Bearer <credential>`, marked sensitive: HTTP/2 does not index it in its header-compression table, and tonic's debug output prints it as `Sensitive` |
| `x-rafn-client-version` | the CLI's `CARGO_PKG_VERSION` |

Both credential kinds are sent the same way. This is the wire contract the backend has to match.

## Token fetch

`ActionsTokenRequest` in `src/config/actions_token.rs` makes one request per CLI invocation, when the backend is built. The token lives for minutes and the CLI for seconds, so it is never refreshed.

- The request is a `GET` on `ACTIONS_ID_TOKEN_REQUEST_URL` with `audience=rafn` appended to its query string. It carries `authorization: bearer <request token>`, `accept: application/json` and `user-agent: rafn/<version>`.
- The URL must be `https://`, or `http://` to a loopback host, as for the API URL.
- Redirects are refused, so the request token cannot be replayed to another host.
- A 10 second timeout covers connecting, TLS and reading the body.
- A non-success status is reported before the body is read.
- A declared `content-length` over 64 KiB is refused. Otherwise the body is read in chunks and abandoned past 64 KiB, because reqwest has no cap of its own.
- The body must be a JSON object with a string `value`, and the value must be three non-empty, dot-separated base64url segments. The signature and claims are the backend's to check.
- Error messages name the host, never the request URL. reqwest prints the URL by default, and it carries job identifiers, so it is stripped from the error chain.

The client is reqwest with default features off and only `rustls-no-provider`: HTTP/1.1 only, no compression, cookies or system proxy. [Transport security](transport-security.md) covers its TLS provider and trust roots.

## Error mapping

`RpcError::from_status` maps every failed RPC:

| gRPC status | Error |
| --- | --- |
| `UNAUTHENTICATED` sent by the service | The credential was rejected. The message names the credential in use. For an API key it says to check the key is correct, unrevoked and issued for this endpoint. For an OIDC token it says to check that the repository owner's account is registered with rafn, or to set `RAFN_API_KEY`. |
| `PERMISSION_DENIED` sent by the service | The credential was accepted, but its organization is not entitled to the operation. The server's reason is repeated verbatim, because only the server knows whether that is an inactive subscription or a used-up quota. |
| any status tonic inferred from a bare HTTP response | Generic failure, as below. |
| anything else, including `RESOURCE_EXHAUSTED` | Generic "gRPC call failed". The status shows only under `{:#}`, which is how the CLI prints errors. |

### Why a bare HTTP 401 or 403 is not a credential error

A response with no `grpc-status` header did not come from the rafn backend. tonic turns its HTTP status into a gRPC status whose message starts with "grpc-status header missing". A proxy or gateway in front of the service that answers 401 or 403 never looked at the credential, so "key rejected" or "key accepted, not entitled" would send the user the wrong way. Such statuses stay generic failures whose chain shows the HTTP status. The prefix is tonic's, not rafn's: a test against a mock that returns a bare 401 and 403 fails if a tonic upgrade rewords it.

`RESOURCE_EXHAUSTED` is not mapped, so a quota refusal the server reports with that code reads as a generic failure. Widen `RpcError` if the service starts using it.

## The GitHub Action's `api-key` input

`action.yml` exports the input as `RAFN_API_KEY` only when it is non-empty, so the input's empty default does not clobber a key set at job or workflow level. An empty input is not exported as an empty variable either. An omitted input and a secret withheld from a fork pull request look identical to the action, so rafn's "not set" message, which names fork pull requests, covers both.

## Why environment variables only

`rafn.toml` and the user config are committed, synced and pasted into bug reports. The environment is what CI secrets already use. The key is read on every invocation, so rotating it needs no CLI change.

## Why no `rafn login`

CI is the primary consumer, and the environment is what CI secrets already use. rafn has no login command and no credential store.

## Why reqwest for the token request

- tonic's channel speaks only HTTP/2, which the token endpoint need not offer.
- A hand-written GET on `hyper` and `tokio-rustls` would add no packages, but would put TLS setup and HTTP edge cases in this repository.
- reqwest's `rustls-no-provider` backend keeps aws-lc-rs out of the build, as [Transport security](transport-security.md) requires.

reqwest brings `hyper-rustls`, `rustls-platform-verifier` and a handful of crates behind them into the lockfile, most of them Android or wasm32 only and never compiled for a release target. `deny.toml` allows `CDLA-Permissive-2.0` for one of those, `webpki-root-certs`.

## Limits

- No `HTTPS_PROXY` support. reqwest's `system-proxy` feature is off, and the gRPC channel has no proxy support either, so a proxied runner cannot reach the service. Turn both on together if that is needed.
- Pull requests from forks get neither secrets nor `id-token: write`, so they cannot reach the cloud service. The error says so.
- The CLI cannot tell a token the backend will accept from one it will not, such as one for a wrong repository owner or an unregistered account. That surfaces as the backend's `UNAUTHENTICATED` message.
- The audience `rafn` is part of the service API. Changing it is a coordinated change.
