/// A simple fibonacci function for benchmarking
pub fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

/// A simple iterative fibonacci for comparison
pub fn fibonacci_iterative(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut a = 0u64;
    let mut b = 1u64;
    for _ in 1..n {
        let tmp = a + b;
        a = b;
        b = tmp;
    }
    b
}
