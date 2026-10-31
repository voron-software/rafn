//! End-to-end tests of the remote backend against an in-process gRPC server.
//!
//! The generated code is client-only (`no_server` in `buf.gen.yaml`), so the
//! mock hand-rolls just enough of a tonic service to answer every unary RPC
//! and to record the metadata each call arrived with. That makes the wire
//! contract (credentials and version on every RPC) a test, not a convention.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use anyhow::{Context, Result};
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
use crate::config::BackendType;
use crate::proto::pb::{
    GetBenchmarkTrendResponse, GetCommitBenchmarksResponse, GetRepositoryTrendsResponse,
    PushResultsResponse,
};

const TEST_KEY: &str = "rk_test_0123456789";

/// What one RPC looked like when it reached the server.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    rpc: &'static str,
    authorization: Option<String>,
    client_version: Option<String>,
}

/// Answers every RPC with an empty success, or with `failure` if set.
#[derive(Clone)]
struct MockService {
    seen: Arc<Mutex<Vec<Seen>>>,
    failure: Option<Status>,
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

        std::future::ready(match &self.service.failure {
            Some(status) => Err(status.clone()),
            None => Ok(Response::new(self.response.clone())),
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
        // Port 0 plus `local_addr` rather than a fixed port, so parallel tests
        // and CI runners never collide.
        let incoming = TcpIncoming::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
        let addr = incoming.local_addr()?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = MockService {
            seen: Arc::clone(&seen),
            failure,
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

fn reader(endpoint: &str) -> Result<RemoteBackend> {
    RemoteBackend::from_effective_with_key(effective(endpoint), ApiKey::parse(TEST_KEY))
}

fn pusher(endpoint: &str) -> Result<RemoteBackend> {
    RemoteBackend::for_push_with_key(effective(endpoint), ApiKey::parse(TEST_KEY))
}

fn trend_query(benchmark_name: Option<&str>) -> TrendQuery {
    TrendQuery {
        benchmark_name: benchmark_name.map(str::to_string),
        limit: 10,
    }
}

/// Run every RPC the CLI can issue against `endpoint`, returning each result.
async fn call_every_rpc(endpoint: &str) -> Result<Vec<(&'static str, Result<()>)>> {
    let reader = reader(endpoint)?;
    let pusher = pusher(endpoint)?;

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

    reader(&endpoint)?.benchmarks_for_commit("abc").await?;

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

    let error = reader(&server.url())?
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
async fn missing_key_fails_before_connecting() -> Result<()> {
    // A listener that stays open: any connection attempt would complete the
    // TCP handshake in the backlog and make `accept` return immediately.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);

    for expected in [
        ApiKeyError::Missing,
        ApiKeyError::Empty,
        ApiKeyError::Malformed,
    ] {
        let error = RemoteBackend::from_effective_with_key(effective(&endpoint), Err(expected))
            .err()
            .context("read backend should not build without a key")?;
        assert_eq!(error.downcast_ref::<ApiKeyError>(), Some(&expected));
        assert!(error.to_string().contains("RAFN_API_KEY"), "{error:#}");

        let error = RemoteBackend::for_push_with_key(effective(&endpoint), Err(expected))
            .err()
            .context("push backend should not build without a key")?;
        assert_eq!(error.downcast_ref::<ApiKeyError>(), Some(&expected));
    }

    let connected = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(connected.is_err(), "something connected to the server");
    Ok(())
}

#[test]
fn the_missing_key_message_is_actionable_for_the_user() {
    let message = ApiKeyError::Missing.to_string();
    assert!(message.contains("export RAFN_API_KEY"), "{message}");
}

#[test]
fn unsupported_schemes_are_rejected_when_the_backend_is_built() -> Result<()> {
    for endpoint in ["grpc://host:50051", "ftp://host", "localhost:50051"] {
        let error = reader(endpoint)
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
async fn https_never_falls_back_to_plaintext() -> Result<()> {
    // A plaintext server cannot complete a TLS handshake, so an `https://`
    // connection to it must fail, and the key must never reach it.
    let server = MockServer::start(None)?;
    let endpoint = server.url().replacen("http", "https", 1);

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        reader(&endpoint)?.benchmarks_for_commit("abc"),
    )
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
    let backend = reader(&closed)?;

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
