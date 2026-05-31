# C# BenchmarkDotNet Example

This example shows how `rafn bench` runs a C# BenchmarkDotNet project.

## Run Locally

```bash
cd examples/csharp/benchmarkdotnet
rafn bench
```

`rafn bench` detects the BenchmarkDotNet project, runs `dotnet run -c Release`,
parses the full JSON report from `BenchmarkDotNet.Artifacts/results/`, saves a
local snapshot in `.rafn/snapshots/`, and compares it with the previous local
snapshot when one exists.

Pass BenchmarkDotNet arguments after `--`:

```bash
rafn bench -- --filter '*Fibonacci*'
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
docker build -f examples/csharp/benchmarkdotnet/Dockerfile -t rafn-example-bdn .
docker run --rm -v "$(pwd)/.rafn-bdn:/app/.rafn" rafn-example-bdn
```

The mounted `.rafn-bdn` directory keeps snapshots between container runs so the
next run can compare against the previous snapshot. Override the default example
metadata with environment variables:

```bash
docker run --rm \
  -e RAFN_REPO=myorg/myrepo \
  -e RAFN_COMMIT=$(git rev-parse HEAD) \
  -v "$(pwd)/.rafn-bdn:/app/.rafn" \
  rafn-example-bdn
```

## Project Structure

- `BenchmarkDotNetExample.csproj` - .NET 8 project with BenchmarkDotNet
- `Program.cs` - Fibonacci benchmark implementation
- `Dockerfile` - containerized `rafn bench` workflow

BenchmarkDotNet writes raw full JSON reports under
`BenchmarkDotNet.Artifacts/results/`; Rafn stores parsed snapshots under
`.rafn/snapshots/`.
