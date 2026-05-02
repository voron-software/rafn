# Rust/Criterion Benchmark Example

This example demonstrates how to use PerfScope with Criterion benchmarks.

## Running the Benchmark

```bash
cd examples/rust/criterion
cargo bench
```

This generates results in `target/criterion/`.

## Ingesting into PerfScope

After running the benchmark, ingest results using the CLI:

```bash
# From the criterion example directory
perfscope ingest --criterion-dir target/criterion

# Or specify repository and commit
perfscope ingest \
  --repository myorg/myrepo \
  --commit-sha $(git rev-parse HEAD) \
  --criterion-dir target/criterion
```

## Benchmark Structure

- `src/lib.rs` - Functions to benchmark
- `benches/sample_bench.rs` - Criterion benchmark definitions

## Output Format

Criterion outputs JSON files to `target/criterion/<benchmark>/new/`:
- `estimates.json` - Statistical estimates (mean, median, std_dev)
- `benchmark.json` - Benchmark metadata

PerfScope parses these files and extracts the timing data.
