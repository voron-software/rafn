# Java JMH Example

This example shows how `rafn bench` runs a Java JMH benchmark project.

## Run Locally

```bash
cd examples/java/jmh
rafn bench
```

`rafn bench` detects the JMH Maven project, runs `mvn package`, runs the generated
`target/benchmarks.jar` with JSON output enabled, parses the result, saves a local
snapshot in `.rafn/snapshots/`, and compares it with the previous local snapshot
when one exists.

Pass JMH arguments after `--`:

```bash
rafn bench -- "FibonacciBenchmark.*" -wi 1 -i 2
```

When running outside a git checkout, provide the repository and commit explicitly:

```bash
RAFN_REPO=myorg/myrepo RAFN_COMMIT=$(git rev-parse HEAD) rafn bench
```

Upload the saved snapshot separately:

```bash
rafn push
```

## Docker Usage

Build from the repository root so the Dockerfile can compile the local `rafn`
binary:

```bash
docker build -f examples/java/jmh/Dockerfile -t rafn-example-jmh .
docker run --rm -v "$(pwd)/.rafn-jmh:/app/.rafn" rafn-example-jmh
```

The mounted `.rafn-jmh` directory keeps snapshots between container runs so the
next run can compare against the previous snapshot. Override the default example
metadata with environment variables:

```bash
docker run --rm \
  -e RAFN_REPO=myorg/myrepo \
  -e RAFN_COMMIT=$(git rev-parse HEAD) \
  -v "$(pwd)/.rafn-jmh:/app/.rafn" \
  rafn-example-jmh
```

## Project Structure

- `pom.xml` - Maven project configuration with JMH dependencies
- `src/main/java/com/perfscope/example/FibonacciBenchmark.java` - Fibonacci benchmarks
- `Dockerfile` - containerized `rafn bench` workflow

JMH writes raw JSON to `.rafn/bench-results/jmh-result.json`; Rafn stores parsed
snapshots under `.rafn/snapshots/`.
