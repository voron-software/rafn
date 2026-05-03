use anyhow::{Context, Result};
use proto::pb::{Benchmark, IngestRequest, ingest_service_client::IngestServiceClient};
use tonic::transport::Channel;

pub struct IngestClient {
    inner: IngestServiceClient<Channel>,
}

impl IngestClient {
    pub async fn connect(grpc_url: String) -> Result<Self> {
        let inner = IngestServiceClient::connect(grpc_url)
            .await
            .context("Failed to connect to gRPC ingest service")?;
        Ok(Self { inner })
    }

    pub async fn submit(&mut self, records: Vec<proto::Benchmark>) -> Result<u32> {
        let pb_records: Vec<Benchmark> = records.into_iter().map(Benchmark::from).collect();
        let response = self
            .inner
            .ingest(IngestRequest {
                records: pb_records,
            })
            .await
            .context("gRPC ingest call failed")?;
        Ok(response.into_inner().benchmarks_ingested)
    }
}
