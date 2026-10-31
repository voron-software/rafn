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
  roots, `http` is plaintext and exists for local development servers, and any
  other scheme is rejected with a message naming the two valid ones. There is
  no way to ask for "https but skip verification", and an `https` URL never
  falls back to plaintext.

## Consequences

- A host with no CA bundle (a bare container) cannot reach an `https` API until
  `ca-certificates` is installed. The failure is tonic's, at connect time.
- `cargo deny` needs `ISC` in the allowed licenses (ring, rustls-webpki,
  untrusted), and the lockfile gains a second `getrandom` and a second
  `windows-sys` version, which `cargo deny check bans` reports as duplicate-version warnings.
- Tests cannot complete a real TLS handshake without a certificate fixture.
  They pin that `https` attempts TLS (it fails against a plaintext server and
  sends nothing), not that a verified handshake succeeds.
