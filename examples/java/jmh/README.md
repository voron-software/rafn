# JMH Benchmark Example

This example demonstrates how to use JMH (Java Microbenchmark Harness) to benchmark Fibonacci implementations and export results in JSON format for perfscope.

## Project Structure

- `pom.xml`: Maven project configuration with JMH dependencies
- `src/main/java/com/perfscope/example/FibonacciBenchmark.java`: JMH benchmark comparing recursive and iterative Fibonacci implementations
- `Dockerfile`: Multi-stage build for containerized benchmark execution

## Benchmark Configuration

The benchmark tests two Fibonacci implementations:
- `recursiveFibonacci`: Classic recursive implementation
- `iterativeFibonacci`: Iterative implementation with O(n) time complexity

Benchmark parameters:
- Input values (n): 10
- Mode: Average time
- Time unit: Nanoseconds
- Fork: 1
- Warmup: 1 iteration × 1s
- Measurement: 2 iterations × 1s

## Building and Running

### Local Build

```bash
mvn clean package
java -jar target/benchmarks.jar -rf json -rff results.json
```

### Docker Build

```bash
docker build -t jmh-benchmark .
```

### Docker Run

```bash
docker run --rm -v $(pwd)/results:/results jmh-benchmark
```

The benchmark results will be saved to `results/jmh-result.json` in JSON format.

## Output Format

JMH generates JSON output containing:
- Benchmark names and parameters
- Performance measurements (mean, error, percentiles)
- JVM and runtime information
- Fork and iteration details

This JSON output can be ingested by perfscope for analysis and visualization.
