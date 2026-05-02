FROM rust:1.85-slim

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY benches ./benches

# Pre-build to cache dependencies
RUN cargo bench --no-run

RUN mkdir -p /results

CMD ["sh", "-c", "cargo bench && cp -r target/criterion/* /results/"]
