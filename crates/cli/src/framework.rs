//! Benchmark framework detection.

use std::path::PathBuf;

/// Supported benchmark frameworks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    Criterion,
}

impl std::fmt::Display for Framework {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Framework::Criterion => write!(f, "criterion"),
        }
    }
}

/// Strategy for discovering benchmark results.
#[derive(Debug, Clone)]
pub enum ResultsStrategy {
    /// Scan a directory for results files.
    Directory(PathBuf),
}

/// Configuration for a detected framework.
#[derive(Debug, Clone)]
pub struct FrameworkConfig {
    pub framework: Framework,
    pub results_strategy: ResultsStrategy,
}

/// Detect benchmark framework from the command being executed.
pub fn detect_framework(command: &[String]) -> Option<FrameworkConfig> {
    let cmd_str = command.join(" ");

    // Criterion: cargo bench
    if cmd_str.contains("cargo") && cmd_str.contains("bench") {
        return Some(FrameworkConfig {
            framework: Framework::Criterion,
            results_strategy: ResultsStrategy::Directory(PathBuf::from("target/criterion")),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_cargo_bench() {
        let cmd = vec!["cargo".to_string(), "bench".to_string()];
        let config = detect_framework(&cmd).unwrap();
        assert_eq!(config.framework, Framework::Criterion);
    }

    #[test]
    fn test_detect_cargo_bench_with_args() {
        let cmd = vec![
            "cargo".to_string(),
            "bench".to_string(),
            "--".to_string(),
            "fibonacci".to_string(),
        ];
        let config = detect_framework(&cmd).unwrap();
        assert_eq!(config.framework, Framework::Criterion);
    }

    #[test]
    fn test_unknown_command() {
        let cmd = vec!["./run-tests.sh".to_string()];
        assert!(detect_framework(&cmd).is_none());
    }
}
