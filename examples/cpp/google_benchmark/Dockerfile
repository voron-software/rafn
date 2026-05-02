# Build stage
FROM ubuntu:22.04 AS builder

RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY CMakeLists.txt .
COPY fibonacci_benchmark.cpp .

RUN cmake -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build --parallel

# Runtime stage
FROM ubuntu:22.04

RUN apt-get update && apt-get install -y \
    libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/build/fibonacci_benchmark .

RUN mkdir -p /results

CMD ["./fibonacci_benchmark", "--benchmark_format=json", "--benchmark_out=/results/gbench-result.json"]
