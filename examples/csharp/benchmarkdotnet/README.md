# BenchmarkDotNet Example for PerfScope

This example demonstrates how to use BenchmarkDotNet with PerfScope to benchmark C# code.

## Project Structure

- `BenchmarkDotNetExample.csproj`: .NET 8 console project with BenchmarkDotNet 0.14.0
- `Program.cs`: Fibonacci benchmark implementation (recursive vs iterative)
- `Dockerfile`: Multi-stage Docker build configuration

## Benchmarks

The example includes two benchmark methods:

1. **RecursiveFibonacci**: Naive recursive implementation
2. **IterativeFibonacci**: Optimized iterative implementation

Both methods are tested with parameter values: 10, 20, and 30.

## Building and Running

### Build Docker Image

```bash
docker build -t perfscope-benchmarkdotnet examples/csharp/benchmarkdotnet
```

### Run Benchmark

```bash
docker run --rm perfscope-benchmarkdotnet
```

### Access Results

BenchmarkDotNet generates results in the `BenchmarkDotNet.Artifacts/results/` directory within the container. The full JSON export is written to `*-report-full.json`.

To extract results from a container:

```bash
# Run with a mounted volume
docker run --rm -v $(pwd)/results:/app/BenchmarkDotNet.Artifacts/results perfscope-benchmarkdotnet
```

## JSON Export Format

The benchmark uses `JsonExporter.Full` which includes the Statistics block required by PerfScope. This provides detailed performance metrics including:

- Mean execution time
- Standard deviation
- Min/Max values
- Percentiles (P25, P50, P75, P95, P99)
- And more

## Local Development

To run without Docker:

```bash
cd examples/csharp/benchmarkdotnet
dotnet restore
dotnet run -c Release
```

Results will be written to `BenchmarkDotNet.Artifacts/results/` in the current directory.
