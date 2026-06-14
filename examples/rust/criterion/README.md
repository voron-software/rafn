# Rust Criterion Example

This example shows how `rafn bench` runs a Rust Criterion benchmark project.

## Run Locally

```bash
cd examples/rust/criterion
rafn bench
```

`rafn bench` detects `Cargo.toml`, runs `cargo bench`, parses Criterion output from
`target/criterion/`, saves a local snapshot in `.rafn/snapshots/`, and compares it
with the previous local snapshot when one exists.

Pass Criterion or Cargo benchmark arguments after `--`:

```bash
rafn bench -- --bench sample_bench
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
docker build -f examples/rust/criterion/Dockerfile -t rafn-example-rust .
docker run --rm -v "$(pwd)/.rafn-rust:/app/.rafn" rafn-example-rust
```

The mounted `.rafn-rust` directory keeps snapshots between container runs so the
next run can compare against the previous snapshot. Override the default example
metadata with environment variables:

```bash
docker run --rm \
  -e RAFN_PROJECT__REPOSITORY__OWNER=myorg \
  -e RAFN_PROJECT__REPOSITORY__REPOSITORY=myrepo \
  -e RAFN_COMMIT=$(git rev-parse HEAD) \
  -v "$(pwd)/.rafn-rust:/app/.rafn" \
  rafn-example-rust
```

## Project Structure

- `src/lib.rs` - functions being benchmarked
- `benches/sample_bench.rs` - Criterion benchmark definitions

Criterion writes raw results under `target/criterion/`; Rafn stores parsed
snapshots under `.rafn/snapshots/`.
