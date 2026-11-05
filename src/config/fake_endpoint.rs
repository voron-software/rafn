//! A throwaway HTTP/1.1 server standing in for GitHub's OIDC token endpoint in
//! tests.
//!
//! A plain `std::net::TcpListener` on a thread rather than a mock framework:
//! the client under test is a hand-rolled GET, so what matters is the exact
//! bytes it sends (request line, headers) and how it copes with awkward
//! answers (error statuses, redirects, oversized or never-ending replies).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};

/// What the server received for one request. Header names are lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeenRequest {
    pub request_line: String,
    headers: Vec<(String, String)>,
}

impl SeenRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// How the server answers every request.
#[derive(Debug, Clone)]
pub(crate) enum Reply {
    /// A complete response.
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Accept the connection, read the request, then say nothing.
    Hang,
}

impl Reply {
    pub fn json(body: impl Into<Vec<u8>>) -> Self {
        Self::Respond {
            status: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: body.into(),
        }
    }

    pub fn status(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::Respond {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn redirect(location: &str) -> Self {
        Self::Respond {
            status: 302,
            headers: vec![("Location".to_string(), location.to_string())],
            body: Vec::new(),
        }
    }
}

pub(crate) struct FakeEndpoint {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeEndpoint {
    pub fn start(reply: Reply) -> Result<Self> {
        // Port 0 so parallel tests never collide.
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread = thread::spawn({
            let seen = Arc::clone(&seen);
            let stop = Arc::clone(&stop);
            move || serve(&listener, &reply, &seen, &stop)
        });

        Ok(Self {
            addr,
            seen,
            stop,
            thread: Some(thread),
        })
    }

    /// `http://127.0.0.1:<port><path_and_query>`.
    pub fn url(&self, path_and_query: &str) -> String {
        format!("http://{}{path_and_query}", self.addr)
    }

    pub fn requests(&self) -> Result<Vec<SeenRequest>> {
        let seen = self
            .seen
            .lock()
            .map_err(|_| anyhow::anyhow!("fake endpoint state poisoned"))?;
        Ok(seen.clone())
    }

    /// A URL on a port nothing listens on, for connection-refused cases.
    pub fn closed_url() -> Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        drop(listener);
        Ok(format!("http://{addr}/"))
    }
}

impl Drop for FakeEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // A panic in the server thread has nowhere useful to go in a
            // destructor; the test's own assertions report the symptom.
            let _ = thread.join();
        }
    }
}

fn serve(listener: &TcpListener, reply: &Reply, seen: &Mutex<Vec<SeenRequest>>, stop: &AtomicBool) {
    // Connections kept open so a `Hang` reply stays hung until the endpoint
    // is dropped.
    let mut held = Vec::new();

    while !stop.load(Ordering::SeqCst) {
        let Ok((mut stream, _)) = listener.accept() else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };
        if let Ok(request) = read_request(&mut stream) {
            if let Ok(mut seen) = seen.lock() {
                seen.push(request);
            }
            if let Reply::Respond {
                status,
                headers,
                body,
            } = reply
            {
                // The client may already have given up (timeout tests).
                let _ = write_response(&mut stream, *status, headers, body);
            }
        }
        held.push(stream);
    }
}

fn read_request(stream: &mut TcpStream) -> Result<SeenRequest> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut raw = Vec::new();
    let mut chunk = [0u8; 1024];
    while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk)?;
        anyhow::ensure!(read > 0, "client closed before finishing the request head");
        raw.extend_from_slice(&chunk[..read]);
    }

    let head = String::from_utf8(raw).context("request head is not UTF-8")?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().context("empty request")?.to_string();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();

    Ok(SeenRequest {
        request_line,
        headers,
    })
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} Test\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
