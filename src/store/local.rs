//! Local snapshot store.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::proto::Benchmark;

use super::{Backend, TrendDataPoint, TrendQuery};

const SNAPSHOTS_DIR: &str = ".rafn/snapshots";

#[derive(Debug, Clone, Default)]
pub struct LocalBackend {
    root_override: Option<PathBuf>,
}

impl LocalBackend {
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root_override: Some(root.into()),
        }
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.root_override
            .as_ref()
            .map(|r| r.join(SNAPSHOTS_DIR))
            .unwrap_or_else(|| PathBuf::from(SNAPSHOTS_DIR))
    }

    fn snapshot_path(&self, commit: &str) -> PathBuf {
        self.snapshots_dir().join(format!("{commit}.json"))
    }

    pub fn save(&self, commit: &str, benchmarks: &[Benchmark]) -> Result<()> {
        let dir = self.snapshots_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create snapshot directory: {}", dir.display()))?;

        let path = self.snapshot_path(commit);
        let content =
            serde_json::to_string_pretty(benchmarks).context("Failed to serialize benchmarks")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write snapshot: {}", path.display()))?;

        Ok(())
    }

    pub fn load(&self, commit: &str) -> Result<Option<Vec<Benchmark>>> {
        let path = self.snapshot_path(commit);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read snapshot: {}", path.display()))?;
        let benchmarks: Vec<Benchmark> = serde_json::from_str(&content).with_context(|| {
            format!(
                "Failed to parse snapshot ({}): corrupt JSON?",
                path.display()
            )
        })?;
        Ok(Some(benchmarks))
    }

    pub fn list_commits(&self) -> Result<Vec<(String, SystemTime)>> {
        let dir = self.snapshots_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries: Vec<(String, SystemTime)> = std::fs::read_dir(&dir)
            .with_context(|| format!("Failed to read snapshot directory: {}", dir.display()))?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                let name = path.file_name()?.to_str()?.to_owned();
                let commit = name.strip_suffix(".json")?.to_owned();
                let mtime = entry.metadata().ok()?.modified().ok()?;
                Some((commit, mtime))
            })
            .collect();

        entries.sort_by_key(|(_, mtime)| *mtime);
        Ok(entries)
    }

    pub fn previous_before(&self, current_commit: &str) -> Result<Option<Vec<Benchmark>>> {
        let previous = self
            .list_commits()?
            .into_iter()
            .rev()
            .find(|(commit, _)| commit != current_commit);

        match previous {
            Some((commit, _)) => self.load(&commit),
            None => Ok(None),
        }
    }
}

impl Backend for LocalBackend {
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<Benchmark>> {
        self.load(commit_sha)?.ok_or_else(|| {
            anyhow::anyhow!(
                "No local snapshot found for commit '{commit_sha}'. \
                 Run `rafn bench` on that commit first."
            )
        })
    }

    async fn trend(&self, query: TrendQuery) -> Result<Vec<TrendDataPoint>> {
        let mut data_points = Vec::new();

        for (commit, mtime) in &self.list_commits()? {
            let benchmarks = match self.load(commit)? {
                Some(b) => b,
                None => continue,
            };
            let timestamp_ms = mtime
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            for bench in benchmarks {
                if let Some(ref name) = query.benchmark_name
                    && bench.benchmark_name != *name
                {
                    continue;
                }
                data_points.push(TrendDataPoint {
                    benchmark_name: bench.benchmark_name,
                    commit_sha: commit.clone(),
                    timestamp: timestamp_ms,
                    mean_ns: bench.metrics.mean_ns,
                    median_ns: bench.metrics.median_ns,
                    stddev_ns: bench.metrics.stddev_ns,
                });
            }
        }

        Ok(data_points)
    }
}

pub fn default_backend() -> LocalBackend {
    LocalBackend::default()
}

pub fn backend_with_root(root: &Path) -> LocalBackend {
    LocalBackend::with_root(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Benchmark, Metrics};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_benchmark(name: &str, mean_ns: f64) -> Benchmark {
        Benchmark {
            tenant_id: Uuid::nil(),
            repository: "test/repo".to_string(),
            commit_sha: "abc123".to_string(),
            benchmark_name: name.to_string(),
            timestamp: Utc::now(),
            toolset: "criterion".to_string(),
            language: "rust".to_string(),
            branch: None,
            tag: None,
            ci_job_id: None,
            metrics: Metrics {
                mean_ns,
                ..Default::default()
            },
            custom_metrics: Default::default(),
            labels: Default::default(),
            cpu_model: None,
            os: None,
            raw_json: None,
        }
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());

        let benches = vec![make_benchmark("foo", 1_000_000.0)];
        store.save("abc123", &benches).unwrap();

        let loaded = store.load("abc123").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].benchmark_name, "foo");
    }

    #[test]
    fn test_load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());
        let result = store.load("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_previous_before_picks_correct_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());

        store.save("commit1", &[make_benchmark("a", 1.0)]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.save("commit2", &[make_benchmark("a", 2.0)]).unwrap();

        let prev = store.previous_before("commit2").unwrap();
        assert!(prev.is_some());
        let prev = prev.unwrap();
        assert_eq!(prev[0].metrics.mean_ns, 1.0);
    }

    #[test]
    fn test_previous_before_no_other_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());

        store.save("only", &[make_benchmark("a", 1.0)]).unwrap();
        let prev = store.previous_before("only").unwrap();
        assert!(prev.is_none());
    }

    #[test]
    fn test_list_commits_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());
        let commits = store.list_commits().unwrap();
        assert!(commits.is_empty());
    }

    #[tokio::test]
    async fn test_trend_filters_by_benchmark_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());

        store
            .save(
                "commit1",
                &[make_benchmark("kept", 1.0), make_benchmark("filtered", 2.0)],
            )
            .unwrap();

        let points = store
            .trend(TrendQuery {
                benchmark_name: Some("kept".to_string()),
                limit: 50,
            })
            .await
            .unwrap();

        assert_eq!(points.len(), 1);
        assert_eq!(points[0].benchmark_name, "kept");
    }
}
