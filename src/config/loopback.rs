//! The loopback rule shared by every place a bearer credential could leave
//! the machine.

use url::Host;

/// Whether traffic to `host` stays on this machine. Judged on the parsed host
/// rather than the raw string, so `localhost.evil.example` and
/// `http://localhost@evil.example` (whose host is `evil.example`) do not pass.
///
/// Plain `http://` is accepted only for such hosts: the API key and the OIDC
/// token rafn sends are bearer credentials, readable by anyone on the path.
pub(crate) fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        // `url` lowercases domains, so this also matches `LOCALHOST`.
        Host::Domain(name) => *name == "localhost",
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}
