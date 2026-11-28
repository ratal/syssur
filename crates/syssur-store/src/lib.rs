//! Persistence abstraction (`ProjectStore`) with a zero-config SQLite
//! implementation using ANSI-compatible SQL migrations ready for PostgreSQL,
//! plus a pure `.sysml` Git-ready directory materializer.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;
use syssur_model::TraceLink;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git error: {0}")]
    Git(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredFileSnapshot {
    pub project_id: String,
    pub file_path: String,
    pub content: String,
    pub crdt_snapshot: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTraceEdge {
    pub project_id: String,
    pub kind: String,
    pub requirement_id: String,
    pub target_path: String,
    pub file_path: String,
}

#[async_trait]
pub trait ProjectStore: Send + Sync {
    async fn save_file_snapshot(&self, snap: &StoredFileSnapshot) -> Result<(), StoreError>;
    async fn load_file_snapshot(
        &self,
        project_id: &str,
        file_path: &str,
    ) -> Result<Option<StoredFileSnapshot>, StoreError>;
    async fn list_files(&self, project_id: &str) -> Result<Vec<StoredFileSnapshot>, StoreError>;
    async fn replace_trace_links(
        &self,
        project_id: &str,
        links: &[TraceLink],
    ) -> Result<(), StoreError>;
    async fn query_trace_links(&self, project_id: &str) -> Result<Vec<StoredTraceEdge>, StoreError>;
}

/// SQLite-backed store using ANSI-compatible DDL portable to PostgreSQL.
#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let opts = SqliteConnectOptions::from_str(url)?.create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS project_files (
                project_id TEXT NOT NULL,
                file_path TEXT NOT NULL,
                content TEXT NOT NULL,
                crdt_snapshot BLOB NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (project_id, file_path)
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS trace_links (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                requirement_id TEXT NOT NULL,
                target_path TEXT NOT NULL,
                file_path TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Writes canonical `.sysml` files to a target directory (pure Everything-as-Code, zero sidecars).
    pub async fn materialize_to_dir(
        &self,
        project_id: &str,
        target_dir: &Path,
    ) -> Result<BTreeMap<String, String>, StoreError> {
        let files = self.list_files(project_id).await?;
        let mut written = BTreeMap::new();
        for f in files {
            let dest = target_dir.join(&f.file_path);
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&dest, f.content.as_bytes()).await?;
            written.insert(f.file_path, f.content);
        }
        Ok(written)
    }

    /// Materializes pure `.sysml` files into `repo_dir` and creates a Git commit.
    pub async fn commit_git_snapshot(
        &self,
        project_id: &str,
        repo_dir: &Path,
        message: &str,
    ) -> Result<(String, Vec<String>), StoreError> {
        tokio::fs::create_dir_all(repo_dir).await?;
        if !repo_dir.join(".git").exists() {
            run_git(repo_dir, &["init"]).await?;
            run_git(repo_dir, &["config", "user.name", "Syssur Snapshot Bot"]).await?;
            run_git(repo_dir, &["config", "user.email", "syssur@local"]).await?;
        }

        let written = self.materialize_to_dir(project_id, repo_dir).await?;
        let file_list: Vec<String> = written.keys().cloned().collect();
        run_git(repo_dir, &["add", "-A"]).await?;

        let status = run_git(repo_dir, &["status", "--porcelain"]).await?;
        if status.trim().is_empty() {
            let head = run_git(repo_dir, &["rev-parse", "--short", "HEAD"])
                .await
                .unwrap_or_else(|_| "clean".to_string());
            return Ok((head.trim().to_string(), file_list));
        }

        run_git(repo_dir, &["commit", "-m", message]).await?;
        let sha = run_git(repo_dir, &["rev-parse", "--short", "HEAD"]).await?;
        Ok((sha.trim().to_string(), file_list))
    }
}

async fn run_git(repo_dir: &Path, args: &[&str]) -> Result<String, StoreError> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(repo_dir)
        .output()
        .await?;
    if !out.status.success() {
        return Err(StoreError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[async_trait]
impl ProjectStore for SqliteStore {
    async fn save_file_snapshot(&self, snap: &StoredFileSnapshot) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO project_files (project_id, file_path, content, crdt_snapshot, updated_at)
            VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)
            ON CONFLICT(project_id, file_path) DO UPDATE SET
                content = excluded.content,
                crdt_snapshot = excluded.crdt_snapshot,
                updated_at = CURRENT_TIMESTAMP;
            "#,
        )
        .bind(&snap.project_id)
        .bind(&snap.file_path)
        .bind(&snap.content)
        .bind(&snap.crdt_snapshot)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_file_snapshot(
        &self,
        project_id: &str,
        file_path: &str,
    ) -> Result<Option<StoredFileSnapshot>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT project_id, file_path, content, crdt_snapshot
            FROM project_files
            WHERE project_id = ?1 AND file_path = ?2;
            "#,
        )
        .bind(project_id)
        .bind(file_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| StoredFileSnapshot {
            project_id: r.get("project_id"),
            file_path: r.get("file_path"),
            content: r.get("content"),
            crdt_snapshot: r.get("crdt_snapshot"),
        }))
    }

    async fn list_files(&self, project_id: &str) -> Result<Vec<StoredFileSnapshot>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT project_id, file_path, content, crdt_snapshot
            FROM project_files
            WHERE project_id = ?1
            ORDER BY file_path ASC;
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| StoredFileSnapshot {
                project_id: r.get("project_id"),
                file_path: r.get("file_path"),
                content: r.get("content"),
                crdt_snapshot: r.get("crdt_snapshot"),
            })
            .collect())
    }

    async fn replace_trace_links(
        &self,
        project_id: &str,
        links: &[TraceLink],
    ) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM trace_links WHERE project_id = ?1;")
            .bind(project_id)
            .execute(&self.pool)
            .await?;

        for link in links {
            sqlx::query(
                r#"
                INSERT INTO trace_links (project_id, kind, requirement_id, target_path, file_path)
                VALUES (?1, ?2, ?3, ?4, ?5);
                "#,
            )
            .bind(project_id)
            .bind(format!("{:?}", link.kind))
            .bind(link.requirement.as_str())
            .bind(&link.target)
            .bind(&link.span.file)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn query_trace_links(&self, project_id: &str) -> Result<Vec<StoredTraceEdge>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT project_id, kind, requirement_id, target_path, file_path
            FROM trace_links
            WHERE project_id = ?1
            ORDER BY requirement_id ASC;
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| StoredTraceEdge {
                project_id: r.get("project_id"),
                kind: r.get("kind"),
                requirement_id: r.get("requirement_id"),
                target_path: r.get("target_path"),
                file_path: r.get("file_path"),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syssur_model::{ElementId, RelationshipKind, Span};

    #[tokio::test]
    async fn sqlite_store_saves_snapshots_and_trace_links() {
        let store = SqliteStore::connect("sqlite::memory:").await.unwrap();
        let snap = StoredFileSnapshot {
            project_id: "rover".into(),
            file_path: "power.sysml".into(),
            content: "package Power {}".into(),
            crdt_snapshot: vec![1, 2, 3],
        };
        store.save_file_snapshot(&snap).await.unwrap();
        let loaded = store
            .load_file_snapshot("rover", "power.sysml")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded, snap);

        store
            .replace_trace_links(
                "rover",
                &[TraceLink {
                    kind: RelationshipKind::Satisfy,
                    requirement: ElementId::new("PowerBudgetReq"),
                    target: "nominalRover.powertrain".into(),
                    is_asserted: false,
                    span: Span::new("vehicle.sysml", 0, 10),
                }],
            )
            .await
            .unwrap();
        let links = store.query_trace_links("rover").await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].requirement_id, "PowerBudgetReq");

        let temp_repo = std::env::temp_dir().join(format!("syssur_git_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_repo);
        let (sha, files) = store
            .commit_git_snapshot("rover", &temp_repo, "Initial snapshot")
            .await
            .unwrap();
        assert!(!sha.is_empty());
        assert_eq!(files, vec!["power.sysml".to_string()]);
        assert!(temp_repo.join(".git").exists());
        assert!(temp_repo.join("power.sysml").exists());
        let _ = std::fs::remove_dir_all(&temp_repo);
    }
}
