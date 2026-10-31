# 0001: TLS via rustls with ring and the OS trust store

**Status:** Accepted

## Context

The cloud backend is served over `https://`, but tonic ships without TLS
unless a feature is enabled, so until now the CLI could only speak plaintext
HTTP/2. Enabling TLS means choosing a crypto provider and a source of trusted
root certificates, and both choices are visible to every user of a CLI that is
distributed as prebuilt binaries for five targets (including aarch64 Linux and
Windows MSVC) as well as built from source.

## Decision

- Provider: tonic's `tls-ring`. ring needs only a C compiler; `aws-lc-rs`, the
  alternative, can need cmake or NASM depending on the target, which would
  complicate the cross-compiled release matrix.
- Roots: `tls-native-roots`, not a bundled Mozilla list. Developer laptops and
  CI runners behind a TLS-inspecting proxy install the proxy's CA in the OS
  store; a bundled list would reject every connection there.
- Scheme decides transport, in one place (`ServiceUrl` in
  `src/store/remote/channel.rs`): `https` gets TLS verified against those
  roots, `http` is plaintext, and any other scheme is rejected with a message
  naming the two valid ones. There is no way to ask for "https but skip
  verification", and an `https` URL never falls back to plaintext.
- `http` is accepted only when the parsed host is loopback (`localhost`,
  `127.0.0.0/8`, `[::1]`); every other host is rejected with an error that
  points to `https://`. The API key rides on every request as a bearer token,
  and HTTP/2 header compression is an encoding, not encryption, so a plaintext
  connection to a remote host hands the key to anyone on the path, even if the
  server then refuses the request. The host is judged after URL parsing, so
  `http://localhost@evil.example` and `http://localhost.evil.example` do not
  pass, and the channel connects to the parser's normalized spelling of the URL
  (`http://127.1` becomes `http://127.0.0.1/`, fragment and query dropped), so
  the host that was checked is the host that is dialed. There is deliberately no opt-in for plaintext to a non-loopback host
  (an environment flag, say): reach a development server through a loopback
  port forward, or give it TLS.

## Consequences

- A host with no CA bundle (a bare container) cannot reach an `https` API until
  `ca-certificates` is installed. The failure is tonic's, at connect time.
- `cargo deny` needs `ISC` in the allowed licenses (ring, rustls-webpki,
  untrusted), and the lockfile gains a second `getrandom` and a second
  `windows-sys` version, which `cargo deny check bans` reports as duplicate-version warnings.
- Tests cannot complete a real TLS handshake without a certificate fixture and
  a way to inject a test root. They pin that an `https` connection opens with a
  TLS handshake record (a raw listener checks the first bytes) and that it
  never reaches a plaintext server, not that a verified handshake succeeds. The
  first of these needs the native root store to load, so the test fails on a
  host with no CA bundle, like the CLI itself.
