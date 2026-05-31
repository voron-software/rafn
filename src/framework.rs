//! Benchmark framework detection and command planning.

use anyhow::{Context, Result, bail};
use std::fmt;
use std::path::{Path, PathBuf};

/// Supported benchmark frameworks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    RustCriterion,
    DotNetBenchmarkDotNet,
    JavaJmh,
    CppGoogleBenchmark,
}

impl fmt::Display for Framework {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Framework::RustCriterion => write!(f, "rust/criterion"),
            Framework::DotNetBenchmarkDotNet => write!(f, "csharp/benchmarkdotnet"),
            Framework::JavaJmh => write!(f, "java/jmh"),
            Framework::CppGoogleBenchmark => write!(f, "cpp/google_benchmark"),
        }
    }
}

/// A subprocess step needed to run a benchmark framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessCommand {
    pub program: String,
    pub args: Vec<String>,
    pub current_dir: PathBuf,
}

impl ProcessCommand {
    fn new(program: impl Into<String>, args: Vec<String>, current_dir: PathBuf) -> Self {
        Self {
            program: program.into(),
            args,
            current_dir,
        }
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(self.program.clone());
        parts.extend(self.args.clone());
        parts.join(" ")
    }
}

/// Strategy for discovering benchmark results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultsStrategy {
    /// Scan Criterion's target/criterion tree.
    CriterionDirectory(PathBuf),
    /// Read a single JSON result file.
    JsonFile(PathBuf),
    /// Read JSON result files from a directory, optionally filtered by suffix.
    JsonDirectory {
        dir: PathBuf,
        required_suffix: Option<String>,
    },
}

/// Configuration for a detected framework.
#[derive(Debug, Clone)]
pub struct FrameworkConfig {
    pub framework: Framework,
    pub commands: Vec<ProcessCommand>,
    pub results_strategy: ResultsStrategy,
}

/// Detect the benchmark framework for the current project and build its run plan.
pub fn detect_framework(passthrough_args: &[String]) -> Result<FrameworkConfig> {
    detect_framework_from(std::env::current_dir()?, passthrough_args)
}

fn detect_framework_from(start: PathBuf, passthrough_args: &[String]) -> Result<FrameworkConfig> {
    let mut dir = start.as_path();
    loop {
        if dir.join("Cargo.toml").exists() {
            return Ok(rust_criterion(dir.to_path_buf(), passthrough_args));
        }

        if dir.join("pom.xml").exists() {
            let pom = std::fs::read_to_string(dir.join("pom.xml"))
                .context("Failed to read pom.xml while detecting JMH")?;
            if pom.contains("jmh-core") || pom.contains("org.openjdk.jmh") {
                return Ok(java_jmh(dir.to_path_buf(), passthrough_args));
            }
        }

        if dir.join("CMakeLists.txt").exists() {
            let cmake = std::fs::read_to_string(dir.join("CMakeLists.txt"))
                .context("Failed to read CMakeLists.txt while detecting Google Benchmark")?;
            if cmake.contains("benchmark::benchmark") || cmake.contains("googlebenchmark") {
                return cpp_google_benchmark(dir.to_path_buf(), &cmake, passthrough_args);
            }
        }

        let csproj_candidates = benchmarkdotnet_projects_in_dir(dir)?;
        match csproj_candidates.as_slice() {
            [csproj] => return Ok(dotnet_benchmarkdotnet(csproj, passthrough_args)),
            [] => {}
            candidates => {
                let list = candidates
                    .iter()
                    .map(|p| format!("  - {}", p.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                bail!("Multiple BenchmarkDotNet projects found:\n{list}");
            }
        }

        let Some(parent) = dir.parent() else {
            break;
        };
        dir = parent;
    }

    bail!(
        "No supported benchmark framework found. Supported detections: Cargo.toml for Rust Criterion, BenchmarkDotNet .csproj, JMH pom.xml, or CMakeLists.txt using Google Benchmark."
    )
}

fn benchmarkdotnet_projects_in_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("csproj") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if content.contains("BenchmarkDotNet") {
            candidates.push(path);
        }
    }
    candidates.sort();
    Ok(candidates)
}

fn rust_criterion(project_dir: PathBuf, passthrough_args: &[String]) -> FrameworkConfig {
    FrameworkConfig {
        framework: Framework::RustCriterion,
        commands: vec![ProcessCommand::new(
            "cargo",
            append_args(vec!["bench"], passthrough_args),
            project_dir.clone(),
        )],
        results_strategy: ResultsStrategy::CriterionDirectory(project_dir.join("target/criterion")),
    }
}

fn dotnet_benchmarkdotnet(csproj: &Path, passthrough_args: &[String]) -> FrameworkConfig {
    let project_dir = csproj
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    FrameworkConfig {
        framework: Framework::DotNetBenchmarkDotNet,
        commands: vec![ProcessCommand::new(
            "dotnet",
            append_args(
                vec![
                    "run",
                    "-c",
                    "Release",
                    "--project",
                    csproj.to_string_lossy().as_ref(),
                    "--",
                ],
                passthrough_args,
            ),
            project_dir.clone(),
        )],
        results_strategy: ResultsStrategy::JsonDirectory {
            dir: project_dir.join("BenchmarkDotNet.Artifacts/results"),
            required_suffix: Some("-report-full.json".to_string()),
        },
    }
}

fn java_jmh(project_dir: PathBuf, passthrough_args: &[String]) -> FrameworkConfig {
    let result_file = project_dir.join(".rafn/bench-results/jmh-result.json");
    FrameworkConfig {
        framework: Framework::JavaJmh,
        commands: vec![
            ProcessCommand::new("mvn", vec!["package".to_string()], project_dir.clone()),
            ProcessCommand::new(
                "java",
                append_args(
                    vec![
                        "-jar",
                        "target/benchmarks.jar",
                        "-rf",
                        "json",
                        "-rff",
                        result_file.to_string_lossy().as_ref(),
                    ],
                    passthrough_args,
                ),
                project_dir,
            ),
        ],
        results_strategy: ResultsStrategy::JsonFile(result_file),
    }
}

fn cpp_google_benchmark(
    project_dir: PathBuf,
    cmake: &str,
    passthrough_args: &[String],
) -> Result<FrameworkConfig> {
    let executable = first_cmake_executable(cmake)
        .context("Could not infer Google Benchmark executable from add_executable(...)")?;
    let result_file = project_dir.join(".rafn/bench-results/google-benchmark.json");
    let executable_path = format!("./build/{executable}");

    Ok(FrameworkConfig {
        framework: Framework::CppGoogleBenchmark,
        commands: vec![
            ProcessCommand::new(
                "cmake",
                vec![
                    "-B".to_string(),
                    "build".to_string(),
                    "-DCMAKE_BUILD_TYPE=Release".to_string(),
                ],
                project_dir.clone(),
            ),
            ProcessCommand::new(
                "cmake",
                vec![
                    "--build".to_string(),
                    "build".to_string(),
                    "--parallel".to_string(),
                ],
                project_dir.clone(),
            ),
            ProcessCommand::new(
                executable_path,
                append_args(
                    vec![
                        "--benchmark_format=json",
                        &format!("--benchmark_out={}", result_file.to_string_lossy()),
                    ],
                    passthrough_args,
                ),
                project_dir,
            ),
        ],
        results_strategy: ResultsStrategy::JsonFile(result_file),
    })
}

fn append_args(args: Vec<&str>, passthrough_args: &[String]) -> Vec<String> {
    args.into_iter()
        .map(String::from)
        .chain(passthrough_args.iter().cloned())
        .collect()
}

fn first_cmake_executable(cmake: &str) -> Option<String> {
    for line in cmake.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("add_executable") else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('(')?.trim_start();
        let name = rest
            .split(|c: char| c.is_whitespace() || c == ')')
            .next()
            .filter(|name| !name.is_empty())?;
        return Some(name.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn detects_rust_criterion_and_passes_args() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n");

        let config =
            detect_framework_from(tmp.path().to_path_buf(), &["--bench".into(), "core".into()])
                .unwrap();

        assert_eq!(config.framework, Framework::RustCriterion);
        assert_eq!(config.commands[0].program, "cargo");
        assert_eq!(config.commands[0].args, ["bench", "--bench", "core"]);
        assert_eq!(
            config.results_strategy,
            ResultsStrategy::CriterionDirectory(tmp.path().join("target/criterion"))
        );
    }

    #[test]
    fn detects_benchmarkdotnet_project() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("Example.csproj"),
            r#"<PackageReference Include="BenchmarkDotNet" Version="0.14.0" />"#,
        );

        let config = detect_framework_from(tmp.path().to_path_buf(), &["--filter".into()])
            .expect("detect benchmarkdotnet");

        assert_eq!(config.framework, Framework::DotNetBenchmarkDotNet);
        assert_eq!(config.commands[0].program, "dotnet");
        assert!(config.commands[0].args.contains(&"--filter".to_string()));
    }

    #[test]
    fn detects_jmh_project() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("pom.xml"),
            "<artifactId>jmh-core</artifactId>",
        );

        let config = detect_framework_from(tmp.path().to_path_buf(), &[]).unwrap();

        assert_eq!(config.framework, Framework::JavaJmh);
        assert_eq!(config.commands.len(), 2);
        assert_eq!(config.commands[1].args[0], "-jar");
    }

    #[test]
    fn nearest_project_marker_wins() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("Cargo.toml"),
            "[package]\nname = \"root\"\n",
        );
        let nested = tmp.path().join("examples/java");
        write(&nested.join("pom.xml"), "<artifactId>jmh-core</artifactId>");

        let config = detect_framework_from(nested, &[]).unwrap();

        assert_eq!(config.framework, Framework::JavaJmh);
    }

    #[test]
    fn detects_google_benchmark_project() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("CMakeLists.txt"),
            "add_executable(fibonacci_benchmark fibonacci_benchmark.cpp)\ntarget_link_libraries(fibonacci_benchmark benchmark::benchmark_main)",
        );

        let config =
            detect_framework_from(tmp.path().to_path_buf(), &["--benchmark_filter=Fib".into()])
                .unwrap();

        assert_eq!(config.framework, Framework::CppGoogleBenchmark);
        assert_eq!(config.commands.len(), 3);
        assert_eq!(config.commands[2].program, "./build/fibonacci_benchmark");
        assert!(
            config.commands[2]
                .args
                .iter()
                .any(|arg| arg.starts_with("--benchmark_out="))
        );
        assert!(
            config.commands[2]
                .args
                .contains(&"--benchmark_filter=Fib".to_string())
        );
    }

    #[test]
    fn unknown_project_errors() {
        let tmp = TempDir::new().unwrap();
        let err = detect_framework_from(tmp.path().to_path_buf(), &[]).unwrap_err();
        assert!(err.to_string().contains("No supported benchmark framework"));
    }

    #[test]
    fn multiple_benchmarkdotnet_projects_error() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("One.csproj"),
            r#"<PackageReference Include="BenchmarkDotNet" Version="0.14.0" />"#,
        );
        write(
            &tmp.path().join("Two.csproj"),
            r#"<PackageReference Include="BenchmarkDotNet" Version="0.14.0" />"#,
        );

        let err = detect_framework_from(tmp.path().to_path_buf(), &[]).unwrap_err();
        assert!(
            err.to_string()
                .contains("Multiple BenchmarkDotNet projects")
        );
    }
}
