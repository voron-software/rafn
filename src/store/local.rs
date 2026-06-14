//! Local snapshot store.

use anyhow::{Context, Result};
use prost::Message;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::proto::benchmark::{
    statistic_mean_ns, statistic_median_ns, statistic_stddev_ns, timestamp_from_system_time,
    timestamp_to_millis,
};
use crate::proto::pb::{BenchmarkSet, PushResultsRequest};

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
        self.snapshots_dir().join(format!("{commit}.pb"))
    }

    pub fn save(&self, commit: &str, benchmark_sets: &[BenchmarkSet]) -> Result<()> {
        let dir = self.snapshots_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create snapshot directory: {}", dir.display()))?;

        let path = self.snapshot_path(commit);
        let content = PushResultsRequest {
            benchmark_sets: benchmark_sets.to_vec(),
        }
        .encode_to_vec();
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write snapshot: {}", path.display()))?;

        Ok(())
    }

    pub fn load(&self, commit: &str) -> Result<Option<Vec<BenchmarkSet>>> {
        let path = self.snapshot_path(commit);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read(&path)
            .with_context(|| format!("Failed to read snapshot: {}", path.display()))?;
        let request = PushResultsRequest::decode(content.as_slice()).with_context(|| {
            format!(
                "Failed to parse snapshot ({}): corrupt protobuf?",
                path.display()
            )
        })?;
        Ok(Some(request.benchmark_sets))
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
                let commit = name.strip_suffix(".pb")?.to_owned();
                let mtime = entry.metadata().ok()?.modified().ok()?;
                Some((commit, mtime))
            })
            .collect();

        entries.sort_by_key(|(_, mtime)| *mtime);
        Ok(entries)
    }

    pub fn previous_before(&self, current_commit: &str) -> Result<Option<Vec<BenchmarkSet>>> {
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
    async fn benchmarks_for_commit(&self, commit_sha: &str) -> Result<Vec<BenchmarkSet>> {
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
            let benchmark_sets = match self.load(commit)? {
                Some(b) => b,
                None => continue,
            };
            let fallback_timestamp_ms = timestamp_to_millis(&timestamp_from_system_time(*mtime));

            for set in benchmark_sets {
                let timestamp_ms = set
                    .run_started_at
                    .as_ref()
                    .map(timestamp_to_millis)
                    .unwrap_or(fallback_timestamp_ms);
                for bench in set.benchmarks {
                    if let Some(ref name) = query.benchmark_name
                        && bench.name != *name
                    {
                        continue;
                    }
                    let Some(mean_ns) = statistic_mean_ns(&bench) else {
                        continue;
                    };
                    let median_ns = statistic_median_ns(&bench);
                    let stddev_ns = statistic_stddev_ns(&bench);
                    data_points.push(TrendDataPoint {
                        benchmark_name: bench.name,
                        commit_sha: commit.clone(),
                        timestamp: timestamp_ms,
                        mean_ns,
                        median_ns,
                        stddev_ns,
                    });
                }
            }
        }

        Ok(data_points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RepositoryRef;
    use crate::proto::benchmark::{
        benchmark_record, benchmark_set, metric_statistics, statistic_mean_ns,
    };

    fn test_repository() -> RepositoryRef {
        RepositoryRef {
            forge: "github.com".to_string(),
            owner: "test".to_string(),
            repository: "repo".to_string(),
        }
    }

    fn make_set(name: &str, mean_ns: f64) -> BenchmarkSet {
        benchmark_set(
            &test_repository(),
            "abc123",
            None,
            "run-1".to_string(),
            prost_types::Timestamp::default(),
            "rust",
            "criterion",
            vec![benchmark_record(
                name.to_string(),
                metric_statistics(mean_ns, 0.0, 0.0, 0.0, 0.0, None),
            )],
        )
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());

        let benches = vec![make_set("foo", 1_000_000.0)];
        store.save("abc123", &benches).unwrap();

        let loaded = store.load("abc123").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].benchmarks[0].name, "foo");
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

        store.save("commit1", &[make_set("a", 1.0)]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.save("commit2", &[make_set("a", 2.0)]).unwrap();

        let prev = store.previous_before("commit2").unwrap();
        assert!(prev.is_some());
        let prev = prev.unwrap();
        assert_eq!(statistic_mean_ns(&prev[0].benchmarks[0]), Some(1.0));
    }

    #[test]
    fn test_previous_before_no_other_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalBackend::with_root(dir.path());

        store.save("only", &[make_set("a", 1.0)]).unwrap();
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
                &[make_set("kept", 1.0), make_set("filtered", 2.0)],
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
