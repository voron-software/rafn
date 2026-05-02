#include <benchmark/benchmark.h>

static long long FibonacciRecursive(int n) {
    if (n <= 1) return n;
    return FibonacciRecursive(n - 1) + FibonacciRecursive(n - 2);
}

static long long FibonacciIterative(int n) {
    if (n <= 1) return n;
    long long a = 0, b = 1;
    for (int i = 2; i <= n; ++i) {
        long long c = a + b;
        a = b;
        b = c;
    }
    return b;
}

static void BM_FibonacciRecursive(benchmark::State& state) {
    for (auto _ : state) {
        benchmark::DoNotOptimize(FibonacciRecursive(state.range(0)));
    }
}

static void BM_FibonacciIterative(benchmark::State& state) {
    for (auto _ : state) {
        benchmark::DoNotOptimize(FibonacciIterative(state.range(0)));
    }
}

BENCHMARK(BM_FibonacciRecursive)->Arg(10);
BENCHMARK(BM_FibonacciIterative)->Arg(10);

BENCHMARK_MAIN();
