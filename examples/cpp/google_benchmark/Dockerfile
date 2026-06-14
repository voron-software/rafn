FROM rust:1-trixie AS rafn-builder

WORKDIR /rafn

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY benches ./benches

RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --target x86_64-unknown-linux-musl --bin rafn

FROM rust:1-trixie

RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=rafn-builder /rafn/target/x86_64-unknown-linux-musl/release/rafn /usr/local/bin/rafn

COPY examples/cpp/google_benchmark/CMakeLists.txt .
COPY examples/cpp/google_benchmark/fibonacci_benchmark.cpp .

# Pre-build to cache Google Benchmark and the example binary.
RUN cmake -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build --parallel

ENV RAFN_PROJECT__REPOSITORY__OWNER=rafn
ENV RAFN_PROJECT__REPOSITORY__REPOSITORY=examples
ENV RAFN_COMMIT=docker-example

CMD ["rafn", "bench"]
