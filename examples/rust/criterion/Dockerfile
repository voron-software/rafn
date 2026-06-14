FROM rust:1-trixie AS rafn-builder

WORKDIR /rafn

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY benches ./benches

RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --target x86_64-unknown-linux-musl --bin rafn

FROM rust:1-trixie

WORKDIR /app

COPY --from=rafn-builder /rafn/target/x86_64-unknown-linux-musl/release/rafn /usr/local/bin/rafn

COPY examples/rust/criterion/Cargo.toml examples/rust/criterion/Cargo.lock ./
COPY examples/rust/criterion/src ./src
COPY examples/rust/criterion/benches ./benches

# Pre-build to cache benchmark dependencies.
RUN cargo bench --no-run

ENV RAFN_PROJECT__REPOSITORY__OWNER=rafn
ENV RAFN_PROJECT__REPOSITORY__REPOSITORY=examples
ENV RAFN_COMMIT=docker-example

CMD ["rafn", "bench"]
