// @generated
/// Generated client implementations.
pub mod benchmark_service_client {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    use tonic::codegen::http::Uri;
    /** Primary gRPC service for the benchmark storage backend.
*/
    #[derive(Debug, Clone)]
    pub struct BenchmarkServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl BenchmarkServiceClient<tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> BenchmarkServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> BenchmarkServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::Body>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<
                http::Request<tonic::body::Body>,
            >>::Error: Into<StdError> + std::marker::Send + std::marker::Sync,
        {
            BenchmarkServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        /** Push a batch of benchmark set results (called by the Rafn CLI after a run).
*/
        pub async fn push_results(
            &mut self,
            request: impl tonic::IntoRequest<super::PushResultsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::PushResultsResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/rafn.v1.BenchmarkService/PushResults",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("rafn.v1.BenchmarkService", "PushResults"));
            self.inner.unary(req, path, codec).await
        }
        /** Get the trend for one specific benchmark in a repository.
*/
        pub async fn get_benchmark_trend(
            &mut self,
            request: impl tonic::IntoRequest<super::GetBenchmarkTrendRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetBenchmarkTrendResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/rafn.v1.BenchmarkService/GetBenchmarkTrend",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("rafn.v1.BenchmarkService", "GetBenchmarkTrend"),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Get per-benchmark trend series for all benchmarks in a repository.
*/
        pub async fn get_repository_trends(
            &mut self,
            request: impl tonic::IntoRequest<super::GetRepositoryTrendsRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetRepositoryTrendsResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/rafn.v1.BenchmarkService/GetRepositoryTrends",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("rafn.v1.BenchmarkService", "GetRepositoryTrends"),
                );
            self.inner.unary(req, path, codec).await
        }
        /** Get all benchmark sets recorded at a specific commit in a repository.
*/
        pub async fn get_commit_benchmarks(
            &mut self,
            request: impl tonic::IntoRequest<super::GetCommitBenchmarksRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetCommitBenchmarksResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/rafn.v1.BenchmarkService/GetCommitBenchmarks",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new("rafn.v1.BenchmarkService", "GetCommitBenchmarks"),
                );
            self.inner.unary(req, path, codec).await
        }
    }
}
