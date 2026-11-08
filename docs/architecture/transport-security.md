# Transport security

rafn talks to the cloud service over gRPC on a tonic channel, and fetches the GitHub Actions OIDC token over HTTPS with reqwest. Both use rustls with the ring crypto provider and trust the operating system's certificate store. A bearer credential crosses only an encrypted link or stays on the machine.

## How the URL decides the transport

The API URL is parsed once, by `ServiceUrl` in `src/store/remote/channel.rs`, and its scheme decides the transport:

| Scheme | Host | Transport |
| --- | --- | --- |
| `https` | any | TLS, verified against the OS trust store |
| `http` | loopback: `localhost`, `127.0.0.0/8`, `[::1]` | plaintext, for local development servers |
| `http` | any other host | refused, with an error that points to `https://` |
| anything else | any | refused, with an error naming the two valid schemes |

- There is no way to ask for `https` without certificate verification, and an `https` URL never falls back to plaintext.
- The loopback check in `src/config/loopback.rs` runs on the parsed host, not the raw string. `http://localhost@evil.example`, whose host is `evil.example`, and `http://localhost.evil.example` do not pass.
- The channel connects to the parser's normalized spelling of the URL, with fragment and query dropped. `http://127.1` becomes `http://127.0.0.1/`, so the host that was checked is the host that is dialed, whatever the system resolver would make of the raw text.
- The OIDC token request URL follows the same loopback rule.

## Why no plaintext opt-in for remote hosts

The API key and the OIDC token ride on every request as bearer tokens. HTTP/2 header compression is an encoding, not encryption, so a plaintext connection to a remote host hands the credential to anyone on the path, even if the server then refuses the request. No flag or environment variable allows plaintext to a non-loopback host. Reach a development server through a loopback port forward, or give it TLS.

## Why ring, not aws-lc-rs

rafn ships prebuilt binaries for five targets, including aarch64 Linux and Windows MSVC, and is also built from source. ring needs only a C compiler. aws-lc-rs can need cmake or NASM depending on the target, which would complicate the release matrix.

- tonic uses its `tls-ring` feature.
- reqwest uses `rustls-no-provider`, and `src/config/actions_token.rs` installs the ring provider process-wide just before it builds the client.

aws-lc-rs is not in the dependency tree.

## Why the OS trust store, not a bundled root list

Developer laptops and CI runners behind a TLS-inspecting proxy install the proxy's CA in the OS store, and a bundled Mozilla list would reject every connection there. The two clients reach the store by different code paths:

| Client | Roots from |
| --- | --- |
| gRPC channel (tonic) | `tls-native-roots`, through `rustls-native-certs` |
| OIDC token request (reqwest) | `rustls-platform-verifier`: the same files as `rustls-native-certs` on Linux, the platform verifier APIs on macOS and Windows |

A CA the OS trusts is trusted by both.

## Limits

- A host with no CA bundle, such as a bare container, cannot reach the service until `ca-certificates` is installed.
  - The gRPC channel fails while building its TLS configuration, before dialing, with tonic's "no native certs found".
  - On Linux the OIDC token request fails earlier, when its HTTP client is built, with "No CA certificates were loaded from the system" in the error chain. This happens even for a loopback `http://` token URL.
- `deny.toml` allows the `ISC` license for ring, rustls-webpki and untrusted.
- ring depends on older versions of `getrandom` and `windows-sys` than the rest of the tree, so `cargo deny check bans` warns about duplicate versions.
- Tests cannot complete a verified TLS handshake without a certificate fixture and a way to inject a test root. For both clients they pin that an `https` connection opens with a TLS handshake record, read by a raw listener, and for the gRPC channel that it never reaches a plaintext server.
- On a Linux host with no CA bundle those handshake tests fail like the CLI itself, and so does every OIDC token fetch test, including the loopback `http://` ones.
