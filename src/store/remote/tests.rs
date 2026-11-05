//! End-to-end tests of the remote backend against an in-process gRPC server.
//!
//! The generated code is client-only (`no_server` in `buf.gen.yaml`), so the
//! mock hand-rolls just enough of a tonic service to answer every unary RPC
//! and to record the metadata each call arrived with. That makes the wire
//! contract (credentials and version on every RPC) a test, not a convention.

use std::convert::Infallible;
use std::env::VarError;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tonic::body::Body;
use tonic::codegen::{BoxFuture, Service, http};
use tonic::server::{Grpc, NamedService, UnaryService};
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response};
use tonic_prost::ProstCodec;

use super::channel::ServiceUrlError;
use super::*;
use crate::config::fake_endpoint::{FakeEndpoint, Reply as TokenReply};
use crate::config::{
    ActionsTokenError, ApiKey, ApiKeyError, BackendType, CredentialError, CredentialSource,
};
use crate::proto::pb::{
    GetBenchmarkTrendResponse, GetCommitBenchmarksResponse, GetRepositoryTrendsResponse,
    PushResultsResponse,
};

const TEST_KEY: &str = "rk_test_0123456789";
const TEST_JWT: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJyZXBvOmFjbWUvd2lkZ2V0cyJ9.c2lnbmF0dXJl";

/// What one RPC looked like when it reached the server.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    rpc: &'static str,
    authorization: Option<String>,
    client_version: Option<String>,
}

/// How the mock answers every RPC. One enum rather than independent options so
/// a mock cannot be asked for two contradictory behaviours at once.
#[derive(Clone)]
enum Reply {
    /// An empty success.
    Success,
    /// A gRPC failure carrying `grpc-status`, as the rafn backend sends.
    Failure(Status),
    /// A bare HTTP error with no `grpc-status` at all, as a proxy, WAF or
    /// IAM-gated Cloud Run front end sends before the request reaches rafn.
    BareHttp(http::StatusCode),
}

#[derive(Clone)]
struct MockService {
    seen: Arc<Mutex<Vec<Seen>>>,
    reply: Reply,
}

impl NamedService for MockService {
    const NAME: &'static str = "rafn.v1.BenchmarkService";
}

/// One unary call: records the metadata, then replies with `response` or the
/// configured failure.
struct Handler<Resp> {
    service: MockService,
    rpc: &'static str,
    response: Resp,
}

impl<Req, Resp: Clone> UnaryService<Req> for Handler<Resp> {
    type Response = Resp;
    type Future = std::future::Ready<Result<Response<Resp>, Status>>;

    fn call(&mut self, request: Request<Req>) -> Self::Future {
        let header = |name: &str| {
            request
                .metadata()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };
        if let Ok(mut seen) = self.service.seen.lock() {
            seen.push(Seen {
                rpc: self.rpc,
                authorization: header("authorization"),
                client_version: header("x-rafn-client-version"),
            });
        }

        std::future::ready(match &self.service.reply {
            Reply::Failure(status) => Err(status.clone()),
            // `route` answers these before a handler exists.
            Reply::Success | Reply::BareHttp(_) => Ok(Response::new(self.response.clone())),
        })
    }
}

impl MockService {
    async fn unary<Req, Resp>(
        self,
        request: http::Request<Body>,
        rpc: &'static str,
        response: Resp,
    ) -> http::Response<Body>
    where
        Req: prost::Message + Default + Send + 'static,
        Resp: prost::Message + Clone + Send + Sync + 'static,
    {
        let handler = Handler {
            service: self,
            rpc,
            response,
        };
        Grpc::new(ProstCodec::<Resp, Req>::default())
            .unary(handler, request)
            .await
    }

    async fn route(self, request: http::Request<Body>) -> http::Response<Body> {
        if let Reply::BareHttp(code) = self.reply {
            let mut response = http::Response::new(Body::empty());
            *response.status_mut() = code;
            response.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/html"),
            );
            return response;
        }

        let method = request
            .uri()
            .path()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();

        match method.as_str() {
            "PushResults" => {
                self.unary::<PushResultsRequest, _>(
                    request,
                    "PushResults",
                    PushResultsResponse::default(),
                )
                .await
            }
            "GetCommitBenchmarks" => {
                self.unary::<GetCommitBenchmarksRequest, _>(
                    request,
                    "GetCommitBenchmarks",
                    GetCommitBenchmarksResponse::default(),
                )
                .await
            }
            "GetBenchmarkTrend" => {
                self.unary::<GetBenchmarkTrendRequest, _>(
                    request,
                    "GetBenchmarkTrend",
                    GetBenchmarkTrendResponse::default(),
                )
                .await
            }
            "GetRepositoryTrends" => {
                self.unary::<GetRepositoryTrendsRequest, _>(
                    request,
                    "GetRepositoryTrends",
                    GetRepositoryTrendsResponse::default(),
                )
                .await
            }
            "CompareCommits" => {
                // `report_from_proto` rejects a response without a summary.
                let response = CompareCommitsResponse {
                    summary: Some(ComparisonSummary::default()),
                    ..Default::default()
                };
                self.unary::<CompareCommitsRequest, _>(request, "CompareCommits", response)
                    .await
            }
            _ => Status::unimplemented("not mocked").into_http(),
        }
    }
}

impl Service<http::Request<Body>> for MockService {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let service = self.clone();
        Box::pin(async move { Ok(service.route(request).await) })
    }
}

/// A mock service bound to an ephemeral loopback port for the test's lifetime.
struct MockServer {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    task: JoinHandle<()>,
}

impl MockServer {
    fn start(failure: Option<Status>) -> Result<Self> {
        Self::start_with(failure.map_or(Reply::Success, Reply::Failure))
    }

    fn start_bare_http(code: http::StatusCode) -> Result<Self> {
        Self::start_with(Reply::BareHttp(code))
    }

    fn start_with(reply: Reply) -> Result<Self> {
        // Port 0 plus `local_addr` rather than a fixed port, so parallel tests
        // and CI runners never collide.
        let incoming = TcpIncoming::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
        let addr = incoming.local_addr()?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = MockService {
            seen: Arc::clone(&seen),
            reply,
        };

        let task = tokio::spawn(async move {
            // The result only matters if serving fails, in which case the
            // test's own RPC assertions fail with a clearer message.
            let _ = Server::builder()
                .add_service(service)
                .serve_with_incoming(incoming)
                .await;
        });

        Ok(Self { addr, seen, task })
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn seen(&self) -> Result<Vec<Seen>> {
        let seen = self
            .seen
            .lock()
            .map_err(|_| anyhow::anyhow!("mock server state poisoned"))?;
        Ok(seen.clone())
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn repository() -> RepositoryRef {
    RepositoryRef {
        forge: "github.com".to_string(),
        owner: "acme".to_string(),
        repository: "widgets".to_string(),
    }
}

fn effective(endpoint: &str) -> EffectiveConfig {
    EffectiveConfig {
        backend_type: BackendType::Cloud,
        endpoint: endpoint.to_string(),
        repository: Some(repository()),
        bench_threshold: 5.0,
    }
}

fn api_key_source() -> Result<CredentialSource, CredentialError> {
    Ok(CredentialSource::ApiKey(ApiKey::parse(TEST_KEY)?))
}

async fn reader(endpoint: &str) -> Result<RemoteBackend> {
    RemoteBackend::from_effective_with_source(effective(endpoint), api_key_source()).await
}

async fn pusher(endpoint: &str) -> Result<RemoteBackend> {
    RemoteBackend::for_push_with_source(effective(endpoint), api_key_source()).await
}

/// The environment GitHub Actions gives a job with `id-token: write`, pointed
/// at `token_url`. The variable names are GitHub's contract, so they are
/// spelled out here rather than imported.
fn actions_source(token_url: &str) -> Result<CredentialSource, CredentialError> {
    let token_url = token_url.to_string();
    CredentialSource::from_lookup(move |name| match name {
        "ACTIONS_ID_TOKEN_REQUEST_URL" => Ok(token_url.clone()),
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN" => Ok("request-token".to_string()),
        "GITHUB_ACTIONS" => Ok("true".to_string()),
        _ => Err(VarError::NotPresent),
    })
}

fn jwt_body() -> String {
    format!(r#"{{"value":"{TEST_JWT}"}}"#)
}

fn trend_query(benchmark_name: Option<&str>) -> TrendQuery {
    TrendQuery {
        benchmark_name: benchmark_name.map(str::to_string),
        limit: 10,
    }
}

/// Run every RPC the CLI can issue against `endpoint`, returning each result.
async fn call_every_rpc(endpoint: &str) -> Result<Vec<(&'static str, Result<()>)>> {
    let reader = reader(endpoint).await?;
    let pusher = pusher(endpoint).await?;

    Ok(vec![
        (
            "push",
            async { pusher.connect_push().await?.submit(vec![]).await.map(drop) }.await,
        ),
        (
            "commit",
            reader.benchmarks_for_commit("abc").await.map(drop),
        ),
        (
            "trend",
            reader.trend(trend_query(Some("foo"))).await.map(drop),
        ),
        ("trends", reader.trend(trend_query(None)).await.map(drop)),
        (
            "compare",
            reader.compare_commits("a", "b", 5.0).await.map(drop),
        ),
    ])
}

fn rpc_error(error: &anyhow::Error) -> Result<&RpcError> {
    error
        .downcast_ref::<RpcError>()
        .with_context(|| format!("not an RpcError: {error:#}"))
}

#[tokio::test]
async fn every_rpc_carries_the_key_and_client_version() -> Result<()> {
    let server = MockServer::start(None)?;

    for (name, result) in call_every_rpc(&server.url()).await? {
        result.with_context(|| format!("{name} failed"))?;
    }

    let seen = server.seen()?;
    let rpcs: Vec<_> = seen.iter().map(|call| call.rpc).collect();
    assert_eq!(
        rpcs,
        [
            "PushResults",
            "GetCommitBenchmarks",
            "GetBenchmarkTrend",
            "GetRepositoryTrends",
            "CompareCommits",
        ]
    );
    for call in &seen {
        assert_eq!(
            call.authorization.as_deref(),
            Some("Bearer rk_test_0123456789"),
            "{}",
            call.rpc
        );
        assert_eq!(
            call.client_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
            "{}",
            call.rpc
        );
    }
    Ok(())
}

#[tokio::test]
async fn uppercase_scheme_still_connects() -> Result<()> {
    let server = MockServer::start(None)?;
    let endpoint = server.url().replacen("http", "HTTP", 1);

    reader(&endpoint)
        .await?
        .benchmarks_for_commit("abc")
        .await?;

    assert_eq!(server.seen()?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn unauthenticated_is_reported_as_a_rejected_key_on_every_rpc() -> Result<()> {
    let server = MockServer::start(Some(Status::unauthenticated("key revoked")))?;

    for (name, result) in call_every_rpc(&server.url()).await? {
        let error = result
            .err()
            .with_context(|| format!("{name} should fail"))?;
        assert!(
            matches!(rpc_error(&error)?, RpcError::Unauthenticated { .. }),
            "{name}: {error:#}"
        );
        let message = format!("{error:#}");
        assert!(message.contains("RAFN_API_KEY"), "{name}: {message}");
        assert!(message.contains("key revoked"), "{name}: {message}");
        assert!(message.contains(&server.url()), "{name}: {message}");
    }
    Ok(())
}

#[tokio::test]
async fn permission_denied_surfaces_the_servers_entitlement_message_on_every_rpc() -> Result<()> {
    let reason = "subscription for acme is inactive";
    let server = MockServer::start(Some(Status::permission_denied(reason)))?;

    for (name, result) in call_every_rpc(&server.url()).await? {
        let error = result
            .err()
            .with_context(|| format!("{name} should fail"))?;
        assert!(
            matches!(rpc_error(&error)?, RpcError::PermissionDenied { .. }),
            "{name}: {error:#}"
        );
        let message = format!("{error:#}");
        assert!(message.contains(reason), "{name}: {message}");
        assert!(
            !message.contains("rejected the API key"),
            "{name}: {message}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn other_failures_name_the_rpc_and_keep_the_status_in_the_chain() -> Result<()> {
    let server = MockServer::start(Some(Status::internal("ClickHouse is down")))?;

    let error = reader(&server.url())
        .await?
        .compare_commits("a", "b", 5.0)
        .await
        .err()
        .context("compare should fail")?;

    assert!(
        matches!(
            rpc_error(&error)?,
            RpcError::Failed {
                rpc: "compare_commits",
                ..
            }
        ),
        "{error:#}"
    );
    assert_eq!(
        error.to_string(),
        format!("gRPC compare_commits call to {} failed", server.url())
    );
    assert!(format!("{error:#}").contains("ClickHouse is down"));
    Ok(())
}

#[tokio::test]
async fn missing_credentials_fail_before_connecting() -> Result<()> {
    // A listener that stays open: any connection attempt would complete the
    // TCP handshake in the backlog and make `accept` return immediately.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);

    let failures: [fn() -> CredentialError; 4] = [
        || ApiKeyError::Missing.into(),
        || ApiKeyError::Empty.into(),
        || ApiKeyError::Malformed.into(),
        || CredentialError::IdTokenUnavailable,
    ];
    for failure in failures {
        let expected = failure().to_string();

        let error = RemoteBackend::from_effective_with_source(effective(&endpoint), Err(failure()))
            .await
            .err()
            .context("read backend should not build without a credential")?;
        assert!(
            error.downcast_ref::<CredentialError>().is_some(),
            "{error:#}"
        );
        assert_eq!(error.to_string(), expected);
        assert!(error.to_string().contains("RAFN_API_KEY"), "{error:#}");

        let error = RemoteBackend::for_push_with_source(effective(&endpoint), Err(failure()))
            .await
            .err()
            .context("push backend should not build without a credential")?;
        assert_eq!(error.to_string(), expected);
    }

    let connected = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(connected.is_err(), "something connected to the server");
    Ok(())
}

#[tokio::test]
async fn an_actions_token_is_fetched_once_and_carried_on_every_rpc() -> Result<()> {
    let tokens = FakeEndpoint::start(TokenReply::json(jwt_body()))?;
    let server = MockServer::start(None)?;
    let reader = RemoteBackend::from_effective_with_source(
        effective(&server.url()),
        actions_source(&tokens.url("/oidc?api-version=2.0")),
    )
    .await?;

    reader.benchmarks_for_commit("abc").await?;
    reader.trend(trend_query(Some("foo"))).await?;
    reader.trend(trend_query(None)).await?;
    reader.compare_commits("a", "b", 5.0).await?;
    reader.connect_push().await?.submit(vec![]).await?;

    // The CLI is short-lived and the token lasts minutes, so one request
    // serves every RPC, however many it makes.
    assert_eq!(tokens.requests()?.len(), 1);
    let seen = server.seen()?;
    assert_eq!(seen.len(), 5);
    for call in &seen {
        assert_eq!(
            call.authorization.as_deref(),
            Some(format!("Bearer {TEST_JWT}").as_str()),
            "{}",
            call.rpc
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_api_key_in_the_environment_beats_the_actions_token() -> Result<()> {
    let tokens = FakeEndpoint::start(TokenReply::json(jwt_body()))?;
    let server = MockServer::start(None)?;
    let token_url = tokens.url("/oidc");
    let source = CredentialSource::from_lookup(|name| match name {
        "RAFN_API_KEY" => Ok(TEST_KEY.to_string()),
        "ACTIONS_ID_TOKEN_REQUEST_URL" => Ok(token_url.clone()),
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN" => Ok("request-token".to_string()),
        _ => Err(VarError::NotPresent),
    });

    RemoteBackend::from_effective_with_source(effective(&server.url()), source)
        .await?
        .benchmarks_for_commit("abc")
        .await?;

    assert!(tokens.requests()?.is_empty());
    let seen = server.seen()?;
    assert_eq!(
        seen.first().and_then(|call| call.authorization.as_deref()),
        Some(format!("Bearer {TEST_KEY}").as_str())
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_token_request_stops_the_backend_before_it_connects() -> Result<()> {
    let tokens = FakeEndpoint::start(TokenReply::status(403, "no"))?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);

    let error = RemoteBackend::from_effective_with_source(
        effective(&endpoint),
        actions_source(&tokens.url("/oidc")),
    )
    .await
    .err()
    .context("a refused token request should fail the build")?;

    assert!(
        matches!(
            error.downcast_ref::<CredentialError>(),
            Some(CredentialError::ActionsToken(ActionsTokenError::Rejected {
                status: 403,
                ..
            }))
        ),
        "{error:#}"
    );
    let connected = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(connected.is_err(), "the service was contacted anyway");
    Ok(())
}

#[tokio::test]
async fn a_bad_api_url_is_reported_before_any_token_request() -> Result<()> {
    let tokens = FakeEndpoint::start(TokenReply::json(jwt_body()))?;

    let error = RemoteBackend::for_push_with_source(
        effective("grpc://host:50051"),
        actions_source(&tokens.url("/oidc")),
    )
    .await
    .err()
    .context("an unsupported scheme should be rejected")?;

    assert!(
        error.downcast_ref::<ServiceUrlError>().is_some(),
        "{error:#}"
    );
    assert!(tokens.requests()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_rejected_actions_token_is_not_blamed_on_the_api_key() -> Result<()> {
    let tokens = FakeEndpoint::start(TokenReply::json(jwt_body()))?;
    let server = MockServer::start(Some(Status::unauthenticated("unknown repository")))?;
    let backend = RemoteBackend::from_effective_with_source(
        effective(&server.url()),
        actions_source(&tokens.url("/oidc")),
    )
    .await?;

    let mut errors = vec![
        backend
            .benchmarks_for_commit("abc")
            .await
            .err()
            .context("read RPC should fail")?,
    ];
    errors.push(
        backend
            .connect_push()
            .await?
            .submit(vec![])
            .await
            .err()
            .context("push RPC should fail")?,
    );

    for error in errors {
        assert!(
            matches!(rpc_error(&error)?, RpcError::Unauthenticated { .. }),
            "{error:#}"
        );
        let message = format!("{error:#}");
        assert!(message.contains("GitHub Actions OIDC token"), "{message}");
        assert!(message.contains("unknown repository"), "{message}");
        assert!(!message.contains("rejected the API key"), "{message}");
        assert!(!message.contains(TEST_JWT), "{message}");
    }
    Ok(())
}

#[tokio::test]
async fn debug_output_never_contains_the_actions_token() -> Result<()> {
    let tokens = FakeEndpoint::start(TokenReply::json(jwt_body()))?;
    let backend = RemoteBackend::from_effective_with_source(
        effective("http://127.0.0.1:1"),
        actions_source(&tokens.url("/oidc")),
    )
    .await?;

    for rendered in [format!("{backend:?}"), format!("{backend:#?}")] {
        assert!(!rendered.contains("eyJ"), "leaked: {rendered}");
        assert!(!rendered.contains("c2lnbmF0dXJl"), "leaked: {rendered}");
    }
    Ok(())
}

#[test]
fn the_missing_key_message_is_actionable_for_the_user() {
    let message = ApiKeyError::Missing.to_string();
    assert!(message.contains("export RAFN_API_KEY"), "{message}");
}

#[tokio::test]
async fn unsupported_schemes_are_rejected_when_the_backend_is_built() -> Result<()> {
    for endpoint in ["grpc://host:50051", "ftp://host", "localhost:50051"] {
        let error = reader(endpoint)
            .await
            .err()
            .with_context(|| format!("{endpoint} should be rejected"))?;
        assert!(
            matches!(
                error.downcast_ref::<ServiceUrlError>(),
                Some(ServiceUrlError::UnsupportedScheme { .. })
            ),
            "{endpoint}: {error:#}"
        );
        assert!(error.to_string().contains("https://"), "{error:#}");
    }
    Ok(())
}

#[tokio::test]
async fn bare_http_refusals_are_not_blamed_on_the_key_or_the_subscription() -> Result<()> {
    // tonic maps a response without `grpc-status` to a status from the HTTP
    // code alone (403 becomes PERMISSION_DENIED, 401 UNAUTHENTICATED). That
    // came from something in front of rafn, which never looked at the key, so
    // telling the user their key was accepted or rejected would be false.
    for (code, tonic_code) in [
        (http::StatusCode::FORBIDDEN, tonic::Code::PermissionDenied),
        (http::StatusCode::UNAUTHORIZED, tonic::Code::Unauthenticated),
    ] {
        let server = MockServer::start_bare_http(code)?;

        for (name, result) in call_every_rpc(&server.url()).await? {
            let error = result
                .err()
                .with_context(|| format!("{name} should fail"))?;
            // Guards the premise: if tonic stops inferring these codes, the
            // assertions below would pass without testing anything.
            let RpcError::Failed { status, .. } = rpc_error(&error)? else {
                anyhow::bail!("{code} on {name} should be a generic failure: {error:#}");
            };
            assert_eq!(status.code(), tonic_code, "{code} on {name}");

            let message = format!("{error:#}");
            assert!(
                message.contains(&format!("HTTP status code {}", code.as_u16())),
                "{name}: {message}"
            );
            assert!(!message.contains("API key"), "{name}: {message}");
            assert!(!message.contains("RAFN_API_KEY"), "{name}: {message}");
            assert!(!message.contains("entitled"), "{name}: {message}");
        }
        assert!(
            server.seen()?.is_empty(),
            "a bare HTTP refusal never reaches a handler"
        );
    }
    Ok(())
}

#[tokio::test]
async fn https_opens_with_a_tls_handshake() -> Result<()> {
    // The listener is a raw socket, not a TLS server: all that matters is the
    // first bytes the client puts on the wire. A TLS record starts with
    // content type 0x16 (handshake) and legacy version 0x03xx. Without this,
    // a TLS branch that were dropped or inverted would still produce an error
    // here (tonic refuses `https` without TLS configured) and the suite would
    // stay green while every real `https://` call failed.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("https://{}", listener.local_addr()?);

    let capture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut head = [0u8; 2];
        // `read`, not `read_exact`: a client that sent nothing should fail
        // the assertion below with its own error, not with "early eof".
        let len = socket.read(&mut head).await?;
        anyhow::Ok(head[..len].to_vec())
    });

    // Dropping the accepted socket after the first read ends the client's
    // handshake, so this resolves to an error rather than hanging.
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        reader(&endpoint).await?.benchmarks_for_commit("abc").await
    })
    .await
    .context("client hung after the server closed the connection")?;
    let client_error = result
        .err()
        .map_or_else(|| "none".to_string(), |e| format!("{e:#}"));

    let head = tokio::time::timeout(Duration::from_secs(5), capture)
        .await
        .with_context(|| {
            // Typically no native root certificates: the TLS config fails
            // before any connection is opened.
            format!("the client never sent anything; its error: {client_error}")
        })???;
    assert_eq!(
        head.get(..2),
        Some(&[0x16, 0x03][..]),
        "first bytes were not a TLS handshake record: {head:02x?}; client error: {client_error}"
    );
    Ok(())
}

#[tokio::test]
async fn https_never_falls_back_to_plaintext() -> Result<()> {
    // A plaintext server cannot complete a TLS handshake, so an `https://`
    // connection to it must fail, and the key must never reach it. That `https`
    // really speaks TLS is pinned by `https_opens_with_a_tls_handshake`.
    let server = MockServer::start(None)?;
    let endpoint = server.url().replacen("http", "https", 1);

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        reader(&endpoint).await?.benchmarks_for_commit("abc").await
    })
    .await
    .context("TLS connection to a plaintext server hung")?;

    let error = result
        .err()
        .context("https to a plaintext server succeeded")?;
    assert!(!format!("{error:#}").contains(TEST_KEY));
    assert!(
        server.seen()?.is_empty(),
        "an RPC reached the plaintext server"
    );
    Ok(())
}

#[tokio::test]
async fn connection_errors_and_debug_output_never_contain_the_key() -> Result<()> {
    // Bind then drop to get a loopback port that refuses connections.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        format!("http://{}", listener.local_addr()?)
    };
    let backend = reader(&closed).await?;

    let error = backend
        .benchmarks_for_commit("abc")
        .await
        .err()
        .context("connecting to a closed port should fail")?;

    for rendered in [
        format!("{error}"),
        format!("{error:#}"),
        format!("{error:?}"),
        format!("{backend:?}"),
        format!("{backend:#?}"),
    ] {
        assert!(!rendered.contains(TEST_KEY), "leaked: {rendered}");
    }
    Ok(())
}
