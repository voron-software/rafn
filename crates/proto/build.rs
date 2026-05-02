fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &[
                "../../proto/perfscope/v1/benchmark.proto",
                "../../proto/perfscope/v1/ingest.proto",
            ],
            &["../../proto"],
        )?;
    Ok(())
}
