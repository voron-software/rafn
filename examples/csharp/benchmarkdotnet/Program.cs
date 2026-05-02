using BenchmarkDotNet.Attributes;
using BenchmarkDotNet.Configs;
using BenchmarkDotNet.Jobs;
using BenchmarkDotNet.Running;
using BenchmarkDotNet.Exporters.Json;
using BenchmarkDotNet.Toolchains.InProcess.Emit;

namespace BenchmarkDotNetExample;

public class FibonacciBenchmarks
{
    public int N { get; set; } = 10;

    [Benchmark]
    public long RecursiveFibonacci()
    {
        return FibRecursive(N);
    }

    [Benchmark]
    public long IterativeFibonacci()
    {
        return FibIterative(N);
    }

    private long FibRecursive(int n)
    {
        if (n <= 1) return n;
        return FibRecursive(n - 1) + FibRecursive(n - 2);
    }

    private long FibIterative(int n)
    {
        if (n <= 1) return n;

        long prev = 0;
        long current = 1;

        for (int i = 2; i <= n; i++)
        {
            long next = prev + current;
            prev = current;
            current = next;
        }

        return current;
    }
}

public class Program
{
    public static void Main(string[] args)
    {
        // InProcessEmitToolchain avoids spawning a subprocess to compile/run a harness,
        // which is necessary in Docker containers where the build environment may differ.
        var config = ManualConfig.Create(DefaultConfig.Instance)
            .WithOptions(ConfigOptions.DisableOptimizationsValidator)
            .AddExporter(JsonExporter.Full)
            .AddJob(Job.Default
                .WithToolchain(InProcessEmitToolchain.Instance)
                .WithWarmupCount(1)
                .WithIterationCount(2));

        BenchmarkRunner.Run<FibonacciBenchmarks>(config, args);
    }
}
