# 0002: API key from the environment, attached by a channel interceptor

**Status:** Accepted

## Context

The cloud backend authenticates callers with an API key, and the backend
resolves the tenant from it. The CLI must get the key from somewhere, put it on
every RPC, and tell the user clearly when the key, or what it entitles them to,
is the problem.

## Decision

- The key is read only from `RAFN_API_KEY` (`src/config/api_key.rs`), never from
  `rafn.toml` or the user config, which are committed, synced and pasted into
  bug reports. Tests pin that the `RAFN_` env scan does not pull it into either
  config type.
- `ApiKey` is a newtype validated at construction, with a redacting `Debug` and
  no `Display`. `RemoteBackend` can only be built with one, and building it is
  synchronous, so a missing key or bad URL fails before any socket is opened.
- One `AuthInterceptor` is installed on the channel, so no RPC can forget
  credentials. It sets `authorization: Bearer <key>` (marked sensitive, which
  keeps it out of HTTP/2 header compression and tonic's debug output) and
  `x-rafn-client-version: <CARGO_PKG_VERSION>`, overwriting any caller-supplied
  `authorization`. This is the wire contract the backend has to match.
- `UNAUTHENTICATED` and `PERMISSION_DENIED` get their own errors (`RpcError`).
  The former says the key was rejected and how to check it. The latter says the
  key was accepted but its organization is not entitled to the operation, and
  repeats the server's reason verbatim, because only the server knows whether
  that is an inactive subscription or a used-up quota. Every other status stays
  a generic "call failed" whose status detail shows under `{:#}` only.
- A status tonic made up from a bare HTTP code (a response with no
  `grpc-status`, recognised by tonic's "grpc-status header missing" message
  prefix) is never mapped to either of those errors. A proxy, a WAF or an
  IAM-gated Cloud Run front end answering 401/403 never looked at the key, so
  "key rejected" or "key accepted, check billing" would send the user the wrong
  way. These stay generic failures whose chain shows the HTTP status. The prefix
  is tonic's, not ours: a test against a mock that returns a bare 403/401 fails
  if an upgrade rewords it.
- `rafn push --dry-run` never builds the remote backend, so it needs no key.
- The GitHub Action's `api-key` input is exported as `RAFN_API_KEY` only when
  non-empty, so a key set at job or workflow level is not clobbered by the
  input's empty default. For the same reason an empty input is not exported as
  an empty variable to get the "set but empty" error: an omitted input and a
  secret withheld from a fork PR look identical to the action, so the CLI's
  "not set" message names fork PRs instead.

## Consequences

- No `rafn login` or credential store: CI is the primary consumer, and the
  environment is what CI secrets already use.
- `RESOURCE_EXHAUSTED` is not mapped, so a quota refusal the server reports with
  that code reads as a generic failure. Widen `RpcError` if the backend uses it.
- Key rotation needs no CLI change; the key is read on every invocation.
