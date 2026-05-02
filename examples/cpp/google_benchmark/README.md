# C++ Google Benchmark Example

Fibonacci benchmarks using [Google Benchmark](https://github.com/google/benchmark).

## Structure

- `fibonacci_benchmark.cpp` — recursive and iterative Fibonacci benchmarks
- `CMakeLists.txt` — build configuration using FetchContent to pull Google Benchmark
- `Dockerfile` — multi-stage build producing a minimal runtime image

## Building locally

```bash
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel
./build/fibonacci_benchmark --benchmark_format=json --benchmark_out=result.json
```

## Docker usage

```bash
docker build -t gbench-example .
docker run --rm -v $(pwd)/results:/results gbench-example
cat results/gbench-result.json
```

## Output format

Google Benchmark produces JSON with the following structure:

```json
{
  "context": { "date": "...", "host_name": "...", ... },
  "benchmarks": [
    {
      "name": "BM_FibonacciRecursive/10",
      "run_type": "iteration",
      "iterations": 1234567,
      "real_time": 42.5,
      "cpu_time": 42.3,
      "time_unit": "ns"
    }
  ]
}
```

This format is parsed by `GoogleBenchmarkParser` in the `ingest` crate.
