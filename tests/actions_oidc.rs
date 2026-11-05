//! End-to-end checks of credential selection with the real binary and a real
//! process environment, which the unit tests (that inject a fake lookup)
//! cannot cover: that the documented variables are the ones actually read, and
//! that `--dry-run` and the local backend never reach for a credential.
//!
//! A std `TcpListener` plays GitHub's token endpoint and records what it is
//! asked. The API URL points at a closed loopback port, so a run that got
//! as far as connecting to the service fails with "Failed to connect", which
//! is how these tests tell "authenticated, then the service was unreachable"
//! from "stopped for want of a credential".
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use tempfile::TempDir;

const JWT: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJyZXBvOmFjbWUvd2lkZ2V0cyJ9.c2lnbmF0dXJl";
const REQUEST_TOKEN: &str = "request-token-SECRET";
const API_KEY: &str = "rk_test_SECRET";

struct TokenServer {
    addr: SocketAddr,
    heads: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TokenServer {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let heads = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread = thread::spawn({
            let heads = Arc::clone(&heads);
            let stop = Arc::clone(&stop);
            move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut head = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk) {
                            Ok(n) if n > 0 => head.extend_from_slice(&chunk[..n]),
                            _ => break,
                        }
                    }
                    if let Ok(mut heads) = heads.lock() {
                        heads.push(String::from_utf8_lossy(&head).to_string());
                    }
                    let body = format!(r#"{{"value":"{JWT}"}}"#);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            }
        });

        Ok(Self {
            addr,
            heads,
            stop,
            thread: Some(thread),
        })
    }

    fn url(&self) -> String {
        format!("http://{}/oidc?api-version=2.0", self.addr)
    }

    fn heads(&self) -> Result<Vec<String>> {
        Ok(self
            .heads
            .lock()
            .map_err(|_| anyhow::anyhow!("token server state poisoned"))?
            .clone())
    }
}

impl Drop for TokenServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A project directory with a `rafn.toml` and a home that holds no user config.
struct Project {
    dir: TempDir,
    home: TempDir,
}

impl Project {
    fn new(backend_type: &str) -> Result<Self> {
        let dir = TempDir::new()?;
        // A port that was just free and has no listener: connecting is refused
        // at once. (A fixed low port is not safe to use: where it is filtered
        // rather than refused, the connect attempt hangs for minutes.)
        let closed = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
        std::fs::write(
            dir.path().join("rafn.toml"),
            format!(
                "[backend]\ntype = \"{backend_type}\"\n\
                 [backend.cloud]\napi_url = \"http://{closed}\"\n\
                 [project.repository]\nforge = \"github.com\"\nowner = \"acme\"\n\
                 repository = \"widgets\"\n"
            ),
        )?;
        Ok(Self {
            dir,
            home: TempDir::new()?,
        })
    }

    /// Run `rafn <args>` with exactly `env` (plus a PATH for git).
    fn rafn(&self, args: &[&str], env: &[(&str, &str)]) -> Result<Output> {
        Command::new(env!("CARGO_BIN_EXE_rafn"))
            .args(args)
            .current_dir(self.dir.path())
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path())
            .envs(env.iter().copied())
            .output()
            .context("failed to run rafn")
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn the_runners_token_variables_authenticate_a_cloud_command() -> Result<()> {
    let server = TokenServer::start()?;
    let project = Project::new("cloud")?;
    let url = server.url();

    let output = project.rafn(
        &["trend"],
        &[
            ("GITHUB_ACTIONS", "true"),
            ("ACTIONS_ID_TOKEN_REQUEST_URL", &url),
            ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", REQUEST_TOKEN),
        ],
    )?;

    // Authenticated, then failed only because the API URL is unreachable.
    assert!(!output.status.success());
    let log = stderr(&output);
    assert!(log.contains("Failed to connect"), "{log}");

    let heads = server.heads()?;
    assert_eq!(heads.len(), 1, "{heads:?}");
    let head = heads.first().context("no request")?.to_lowercase();
    assert!(
        head.starts_with("get /oidc?api-version=2.0&audience=rafn http/1.1\r\n"),
        "{head}"
    );
    assert!(
        head.contains(&format!(
            "authorization: bearer {}",
            REQUEST_TOKEN.to_lowercase()
        )),
        "{head}"
    );

    assert!(!log.contains(REQUEST_TOKEN), "{log}");
    assert!(!log.contains(JWT), "{log}");
    Ok(())
}

#[test]
fn an_api_key_in_the_environment_wins_and_no_token_is_requested() -> Result<()> {
    let server = TokenServer::start()?;
    let project = Project::new("cloud")?;
    let url = server.url();

    let output = project.rafn(
        &["trend"],
        &[
            ("RAFN_API_KEY", API_KEY),
            ("GITHUB_ACTIONS", "true"),
            ("ACTIONS_ID_TOKEN_REQUEST_URL", &url),
            ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", REQUEST_TOKEN),
        ],
    )?;

    let log = stderr(&output);
    assert!(log.contains("Failed to connect"), "{log}");
    assert!(server.heads()?.is_empty());
    assert!(!log.contains(API_KEY), "{log}");
    Ok(())
}

#[test]
fn dry_run_push_never_requests_a_token() -> Result<()> {
    let server = TokenServer::start()?;
    let project = Project::new("cloud")?;
    let url = server.url();

    let output = project.rafn(
        &["push", "--dry-run", "--commit", "abc123"],
        &[
            ("GITHUB_ACTIONS", "true"),
            ("ACTIONS_ID_TOKEN_REQUEST_URL", &url),
            ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", REQUEST_TOKEN),
        ],
    )?;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(server.heads()?.is_empty());
    Ok(())
}

#[test]
fn dry_run_push_needs_no_credential_at_all() -> Result<()> {
    let project = Project::new("cloud")?;

    let output = project.rafn(&["push", "--dry-run", "--commit", "abc123"], &[])?;

    assert!(output.status.success(), "{}", stderr(&output));
    Ok(())
}

#[test]
fn the_local_backend_never_requests_a_token() -> Result<()> {
    let server = TokenServer::start()?;
    let project = Project::new("local")?;
    let url = server.url();

    let output = project.rafn(
        &["trend"],
        &[
            ("GITHUB_ACTIONS", "true"),
            ("ACTIONS_ID_TOKEN_REQUEST_URL", &url),
            ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", REQUEST_TOKEN),
        ],
    )?;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(server.heads()?.is_empty());
    Ok(())
}

#[test]
fn github_actions_without_id_token_permission_says_how_to_grant_it() -> Result<()> {
    let project = Project::new("cloud")?;

    let output = project.rafn(&["trend"], &[("GITHUB_ACTIONS", "true")])?;

    assert!(!output.status.success());
    let log = stderr(&output);
    assert!(log.contains("permissions: id-token: write"), "{log}");
    assert!(!log.contains("Failed to connect"), "{log}");
    Ok(())
}

#[test]
fn outside_github_actions_a_missing_key_is_reported() -> Result<()> {
    let project = Project::new("cloud")?;

    let output = project.rafn(&["trend"], &[])?;

    assert!(!output.status.success());
    let log = stderr(&output);
    assert!(log.contains("RAFN_API_KEY is not set"), "{log}");
    assert!(!log.contains("Failed to connect"), "{log}");
    Ok(())
}
