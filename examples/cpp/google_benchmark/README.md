# C++ Google Benchmark Example

This example shows how `rafn bench` runs a C++ Google Benchmark project.

## Run Locally

```bash
cd examples/cpp/google_benchmark
rafn bench
```

`rafn bench` detects `CMakeLists.txt`, configures and builds the project with
CMake, runs the inferred Google Benchmark executable with JSON output enabled,
saves a local snapshot in `.rafn/snapshots/`, and compares it with the previous
local snapshot when one exists.

Pass Google Benchmark arguments after `--`:

```bash
rafn bench -- --benchmark_filter=Fibonacci
```

When running outside a git checkout, provide the repository and commit explicitly:

```bash
RAFN_PROJECT__REPOSITORY__OWNER=myorg RAFN_PROJECT__REPOSITORY__REPOSITORY=myrepo \
  RAFN_COMMIT=$(git rev-parse HEAD) rafn bench
```

Upload the saved snapshot separately:

```bash
rafn push
```

## Docker Usage

Build from the repository root so the Dockerfile can compile the local `rafn`
binary:

```bash
docker build -f examples/cpp/google_benchmark/Dockerfile -t rafn-example-gbench .
docker run --rm -v "$(pwd)/.rafn-gbench:/app/.rafn" rafn-example-gbench
```

The mounted `.rafn-gbench` directory keeps snapshots between container runs so the
next run can compare against the previous snapshot. Override the default example
metadata with environment variables:

```bash
docker run --rm \
  -e RAFN_PROJECT__REPOSITORY__OWNER=myorg \
  -e RAFN_PROJECT__REPOSITORY__REPOSITORY=myrepo \
  -e RAFN_COMMIT=$(git rev-parse HEAD) \
  -v "$(pwd)/.rafn-gbench:/app/.rafn" \
  rafn-example-gbench
```

## Project Structure

- `fibonacci_benchmark.cpp` - recursive and iterative Fibonacci benchmarks
- `CMakeLists.txt` - build configuration using FetchContent for Google Benchmark
- `Dockerfile` - containerized `rafn bench` workflow

Google Benchmark writes raw JSON to `.rafn/bench-results/google-benchmark.json`;
Rafn stores parsed snapshots under `.rafn/snapshots/`.
