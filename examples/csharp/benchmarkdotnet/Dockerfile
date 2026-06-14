FROM rust:1-trixie AS rafn-builder

WORKDIR /rafn

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY benches ./benches

RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --target x86_64-unknown-linux-musl --bin rafn

FROM mcr.microsoft.com/dotnet/sdk:8.0

WORKDIR /app

COPY --from=rafn-builder /rafn/target/x86_64-unknown-linux-musl/release/rafn /usr/local/bin/rafn

COPY examples/csharp/benchmarkdotnet/BenchmarkDotNetExample.csproj .
COPY examples/csharp/benchmarkdotnet/Program.cs .

# Pre-build to cache NuGet dependencies and compile the benchmark project.
RUN dotnet restore && dotnet build -c Release --no-restore

ENV RAFN_PROJECT__REPOSITORY__OWNER=rafn
ENV RAFN_PROJECT__REPOSITORY__REPOSITORY=examples
ENV RAFN_COMMIT=docker-example

CMD ["rafn", "bench"]
