# Framework examples

`examples/` holds one small benchmark project per supported framework. Each shows how `rafn bench` detects the framework, runs it and reads its results.

| Example | Framework | Detected by | `rafn bench` runs | Raw results |
| --- | --- | --- | --- | --- |
| `examples/rust/criterion` | Rust Criterion | `Cargo.toml` | `cargo bench` | `target/criterion/` |
| `examples/java/jmh` | Java JMH | `pom.xml` that mentions `jmh-core` or `org.openjdk.jmh` | `mvn package`, then `java -jar target/benchmarks.jar` with JSON output | `.rafn/bench-results/jmh-result.json` |
| `examples/cpp/google_benchmark` | C++ Google Benchmark | `CMakeLists.txt` that mentions `benchmark::benchmark` or `googlebenchmark` | CMake configure and build, then the first `add_executable` target with JSON output | `.rafn/bench-results/google-benchmark.json` |
| `examples/csharp/benchmarkdotnet` | C# BenchmarkDotNet | a `.csproj` that mentions `BenchmarkDotNet` | `dotnet run -c Release` | the `*-report-full.json` reports under `BenchmarkDotNet.Artifacts/results/` |

Detection starts in the current directory and walks up, checking in the table's order at each level. Two BenchmarkDotNet projects in one directory are an error.

`rafn bench` parses the raw results, saves a snapshot under `.rafn/snapshots/`, and compares it with the latest snapshot saved for a different commit, if there is one.

## Files

| Example | Files |
| --- | --- |
| `rust/criterion` | `src/lib.rs` with the functions under test, `benches/sample_bench.rs` with the Criterion benchmarks, `Dockerfile` |
| `java/jmh` | `pom.xml` with the JMH dependencies, `src/main/java/dev/rafn/example/FibonacciBenchmark.java`, `Dockerfile` |
| `cpp/google_benchmark` | `fibonacci_benchmark.cpp` with recursive and iterative Fibonacci benchmarks, `CMakeLists.txt` fetching Google Benchmark with FetchContent, `Dockerfile` |
| `csharp/benchmarkdotnet` | `BenchmarkDotNetExample.csproj` (.NET 8), `Program.cs` with the Fibonacci benchmark, `Dockerfile` |

Each example also has a `rafn.toml` that selects the local backend.

## Run locally

```bash
cd examples/rust/criterion
rafn bench
```

Arguments after `--` go to the framework:

| Example | Command |
| --- | --- |
| `rust/criterion` | `rafn bench -- --bench sample_bench` |
| `java/jmh` | `rafn bench -- "FibonacciBenchmark.*" -wi 1 -i 2` |
| `cpp/google_benchmark` | `rafn bench -- --benchmark_filter=Fibonacci` |
| `csharp/benchmarkdotnet` | `rafn bench -- --filter '*Fibonacci*'` |

Outside a git checkout, provide the repository and commit explicitly:

```bash
RAFN_PROJECT__REPOSITORY__OWNER=myorg RAFN_PROJECT__REPOSITORY__REPOSITORY=myrepo \
  RAFN_COMMIT=$(git rev-parse HEAD) rafn bench
```

## Upload

The examples' local backend keeps snapshots on disk, so `rafn push` there reports nothing to push. To upload, select the cloud backend and provide a credential as described in [Authentication](authentication.md):

```bash
export RAFN_API_KEY=<your key>
RAFN_BACKEND__TYPE=cloud rafn push
```

## Docker

Build from the repository root, because each Dockerfile compiles the local `rafn` binary before building the example:

```bash
docker build -f examples/rust/criterion/Dockerfile -t rafn-example-rust .
docker run --rm -v "$(pwd)/.rafn-rust:/app/.rafn" rafn-example-rust
```

| Example | Image tag | Snapshot directory on the host |
| --- | --- | --- |
| `rust/criterion` | `rafn-example-rust` | `.rafn-rust` |
| `java/jmh` | `rafn-example-jmh` | `.rafn-jmh` |
| `cpp/google_benchmark` | `rafn-example-gbench` | `.rafn-gbench` |
| `csharp/benchmarkdotnet` | `rafn-example-bdn` | `.rafn-bdn` |

The mounted directory keeps snapshots between container runs. The images set `RAFN_PROJECT__REPOSITORY__OWNER=rafn`, `RAFN_PROJECT__REPOSITORY__REPOSITORY=examples` and `RAFN_COMMIT=docker-example`. Every run with that fixed commit overwrites the same snapshot and has nothing to compare against, so set a distinct `RAFN_COMMIT` per run to compare runs:

```bash
docker run --rm \
  -e RAFN_PROJECT__REPOSITORY__OWNER=myorg \
  -e RAFN_PROJECT__REPOSITORY__REPOSITORY=myrepo \
  -e RAFN_COMMIT=$(git rev-parse HEAD) \
  -v "$(pwd)/.rafn-rust:/app/.rafn" \
  rafn-example-rust
```
