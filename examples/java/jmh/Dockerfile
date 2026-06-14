FROM rust:1-trixie AS rafn-builder

WORKDIR /rafn

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY benches ./benches

RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --target x86_64-unknown-linux-musl --bin rafn

FROM maven:3.9-eclipse-temurin-21

WORKDIR /app

COPY --from=rafn-builder /rafn/target/x86_64-unknown-linux-musl/release/rafn /usr/local/bin/rafn

COPY examples/java/jmh/pom.xml .
COPY examples/java/jmh/src ./src

# Pre-build to cache Maven dependencies and generate target/benchmarks.jar.
RUN mvn package

ENV RAFN_PROJECT__REPOSITORY__OWNER=rafn
ENV RAFN_PROJECT__REPOSITORY__REPOSITORY=examples
ENV RAFN_COMMIT=docker-example

CMD ["rafn", "bench"]
