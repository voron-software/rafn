# Build stage
FROM mcr.microsoft.com/dotnet/sdk:8.0 AS builder

WORKDIR /app

# Copy project files
COPY BenchmarkDotNetExample.csproj .
COPY Program.cs .

# Restore dependencies
RUN dotnet restore

# Build the project
RUN dotnet build -c Release --no-restore

# Runtime stage — BenchmarkDotNet requires the SDK to compile and run benchmark jobs
FROM mcr.microsoft.com/dotnet/sdk:8.0

WORKDIR /app

# Copy built application and project file from builder stage
# BenchmarkDotNet needs the .csproj at runtime to generate its benchmark harness
COPY --from=builder /app/bin/Release/net8.0 .
COPY --from=builder /app/BenchmarkDotNetExample.csproj .

# BenchmarkDotNet will write results to BenchmarkDotNet.Artifacts/results/*-report-full.json
# Run the benchmark
CMD ["dotnet", "BenchmarkDotNetExample.dll"]
