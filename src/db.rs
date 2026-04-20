use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Target {
    pub id: i64,
    pub url: String,
    pub enabled: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Snapshot {
    pub id: i64,
    pub target_id: i64,
    pub url: String,
    pub taken_at: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub local_path: String,
    pub drive_file_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

/// Config exposée via l'API (pas les secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub interval_secs: i64,
    pub tor_socks: String,
    pub drive_folder_id: String,
    pub drive_enabled: bool,
    pub user_agent: String,
    pub http_timeout_secs: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interval_secs: 900,
            tor_socks: "socks5h://tor:9050".into(),
            drive_folder_id: String::new(),
            drive_enabled: false,
            user_agent: "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0"
                .into(),
            http_timeout_secs: 60,
        }
    }
}

pub async fn open(db_url: &str) -> Result<SqlitePool> {
    // SQLite n'applique les clés étrangères que si `PRAGMA foreign_keys = ON`
    // est positionné sur chaque connexion. On le fait ici pour que le
    // `ON DELETE CASCADE` de snapshots.target_id soit honoré.
    let opts = SqliteConnectOptions::from_str(db_url)?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub async fn load_settings(pool: &SqlitePool) -> Result<Settings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;

    let mut s = Settings::default();
    for (k, v) in rows {
        match k.as_str() {
            "interval_secs" => s.interval_secs = v.parse().unwrap_or(s.interval_secs),
            "tor_socks" => s.tor_socks = v,
            "drive_folder_id" => s.drive_folder_id = v,
            "drive_enabled" => s.drive_enabled = v == "1",
            "user_agent" => s.user_agent = v,
            "http_timeout_secs" => s.http_timeout_secs = v.parse().unwrap_or(s.http_timeout_secs),
            _ => {}
        }
    }
    Ok(s)
}

pub async fn save_settings(pool: &SqlitePool, s: &Settings) -> Result<()> {
    let pairs: [(&str, String); 6] = [
        ("interval_secs", s.interval_secs.to_string()),
        ("tor_socks", s.tor_socks.clone()),
        ("drive_folder_id", s.drive_folder_id.clone()),
        (
            "drive_enabled",
            if s.drive_enabled { "1" } else { "0" }.into(),
        ),
        ("user_agent", s.user_agent.clone()),
        ("http_timeout_secs", s.http_timeout_secs.to_string()),
    ];

    let mut tx = pool.begin().await?;
    for (k, v) in pairs {
        sqlx::query(
            "INSERT INTO settings(key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(k)
        .bind(v)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn list_targets(pool: &SqlitePool) -> Result<Vec<Target>> {
    Ok(
        sqlx::query_as::<_, Target>("SELECT * FROM targets ORDER BY id")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn list_enabled_targets(pool: &SqlitePool) -> Result<Vec<Target>> {
    Ok(
        sqlx::query_as::<_, Target>("SELECT * FROM targets WHERE enabled = 1 ORDER BY id")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn add_target(pool: &SqlitePool, url: &str) -> Result<Target> {
    sqlx::query("INSERT INTO targets(url) VALUES (?)")
        .bind(url)
        .execute(pool)
        .await?;
    let t: Target = sqlx::query_as("SELECT * FROM targets WHERE url = ?")
        .bind(url)
        .fetch_one(pool)
        .await?;
    Ok(t)
}

pub async fn delete_target(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM targets WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_target_enabled(pool: &SqlitePool, id: i64, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE targets SET enabled = ? WHERE id = ?")
        .bind(if enabled { 1 } else { 0 })
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_snapshot(pool: &SqlitePool, s: &NewSnapshot<'_>) -> Result<i64> {
    let res = sqlx::query(
        "INSERT INTO snapshots(target_id, url, taken_at, size_bytes, sha256, local_path, drive_file_id, status, error) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(s.target_id)
    .bind(s.url)
    .bind(&s.taken_at)
    .bind(s.size_bytes)
    .bind(s.sha256)
    .bind(s.local_path)
    .bind(s.drive_file_id)
    .bind(s.status)
    .bind(s.error)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

pub struct NewSnapshot<'a> {
    pub target_id: i64,
    pub url: &'a str,
    pub taken_at: String,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub local_path: &'a str,
    pub drive_file_id: Option<&'a str>,
    pub status: &'a str,
    pub error: Option<&'a str>,
}

pub async fn list_snapshots(
    pool: &SqlitePool,
    target_id: Option<i64>,
    limit: i64,
) -> Result<Vec<Snapshot>> {
    let rows = match target_id {
        Some(tid) => {
            sqlx::query_as::<_, Snapshot>(
                "SELECT * FROM snapshots WHERE target_id = ? \
                 ORDER BY taken_at DESC LIMIT ?",
            )
            .bind(tid)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, Snapshot>("SELECT * FROM snapshots ORDER BY taken_at DESC LIMIT ?")
                .bind(limit)
                .fetch_all(pool)
                .await?
        }
    };
    Ok(rows)
}

pub async fn get_snapshot(pool: &SqlitePool, id: i64) -> Result<Option<Snapshot>> {
    Ok(
        sqlx::query_as::<_, Snapshot>("SELECT * FROM snapshots WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Récupère le sha256 du dernier snapshot OK pour une cible donnée (pour dédup).
pub async fn last_sha_for(pool: &SqlitePool, target_id: i64) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT sha256 FROM snapshots \
         WHERE target_id = ? AND status = 'ok' \
         ORDER BY taken_at DESC LIMIT 1",
    )
    .bind(target_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn test_pool() -> (SqlitePool, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("t.db");
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let pool = open(&url).await.expect("open pool");
        (pool, dir)
    }

    #[test]
    fn settings_default_values() {
        let s = Settings::default();
        assert_eq!(s.interval_secs, 900);
        assert_eq!(s.tor_socks, "socks5h://tor:9050");
        assert!(s.drive_folder_id.is_empty());
        assert!(!s.drive_enabled);
        assert!(s.user_agent.contains("Firefox"));
        assert_eq!(s.http_timeout_secs, 60);
    }

    #[tokio::test]
    async fn settings_roundtrip() {
        let (pool, _tmp) = test_pool().await;

        let loaded = load_settings(&pool).await.unwrap();
        assert_eq!(loaded.interval_secs, Settings::default().interval_secs);

        let custom = Settings {
            interval_secs: 42,
            tor_socks: "socks5h://127.0.0.1:9050".into(),
            drive_folder_id: "FOLDER_ID_XYZ".into(),
            drive_enabled: true,
            user_agent: "ua/test".into(),
            http_timeout_secs: 11,
        };
        save_settings(&pool, &custom).await.unwrap();

        let got = load_settings(&pool).await.unwrap();
        assert_eq!(got.interval_secs, 42);
        assert_eq!(got.tor_socks, "socks5h://127.0.0.1:9050");
        assert_eq!(got.drive_folder_id, "FOLDER_ID_XYZ");
        assert!(got.drive_enabled);
        assert_eq!(got.user_agent, "ua/test");
        assert_eq!(got.http_timeout_secs, 11);
    }

    #[tokio::test]
    async fn target_crud_and_toggle() {
        let (pool, _tmp) = test_pool().await;

        let t = add_target(&pool, "https://example.com").await.unwrap();
        assert_eq!(t.url, "https://example.com");
        assert_eq!(t.enabled, 1);

        let listed = list_targets(&pool).await.unwrap();
        assert_eq!(listed.len(), 1);

        set_target_enabled(&pool, t.id, false).await.unwrap();
        let after = list_enabled_targets(&pool).await.unwrap();
        assert!(after.is_empty(), "cible désactivée doit être filtrée");

        set_target_enabled(&pool, t.id, true).await.unwrap();
        assert_eq!(list_enabled_targets(&pool).await.unwrap().len(), 1);

        delete_target(&pool, t.id).await.unwrap();
        assert!(list_targets(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_target_rejects_duplicate_url() {
        let (pool, _tmp) = test_pool().await;
        add_target(&pool, "https://dup.example").await.unwrap();
        let err = add_target(&pool, "https://dup.example").await;
        assert!(err.is_err(), "UNIQUE(url) doit rejeter le doublon");
    }

    #[tokio::test]
    async fn snapshots_insert_list_filter_and_dedup() {
        let (pool, _tmp) = test_pool().await;
        let t1 = add_target(&pool, "https://a.example").await.unwrap();
        let t2 = add_target(&pool, "https://b.example").await.unwrap();

        let ns =
            |tid: i64, url: &'static str, when: &str, sha: &'static str, status: &'static str| {
                NewSnapshot {
                    target_id: tid,
                    url,
                    taken_at: when.to_string(),
                    size_bytes: 10,
                    sha256: sha,
                    local_path: "/tmp/fake.html",
                    drive_file_id: None,
                    status,
                    error: None,
                }
            };

        insert_snapshot(
            &pool,
            &ns(
                t1.id,
                "https://a.example",
                "2026-04-19T10:00:00Z",
                "aaa",
                "ok",
            ),
        )
        .await
        .unwrap();
        insert_snapshot(
            &pool,
            &ns(
                t1.id,
                "https://a.example",
                "2026-04-19T11:00:00Z",
                "bbb",
                "ok",
            ),
        )
        .await
        .unwrap();
        insert_snapshot(
            &pool,
            &ns(
                t2.id,
                "https://b.example",
                "2026-04-19T10:30:00Z",
                "ccc",
                "fetch_error",
            ),
        )
        .await
        .unwrap();

        let all = list_snapshots(&pool, None, 100).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].sha256, "bbb", "tri DESC par taken_at");

        let t1_only = list_snapshots(&pool, Some(t1.id), 100).await.unwrap();
        assert_eq!(t1_only.len(), 2);

        let last_ok = last_sha_for(&pool, t1.id).await.unwrap();
        assert_eq!(last_ok.as_deref(), Some("bbb"));

        // status != 'ok' ne compte pas pour la dédup
        let t2_last = last_sha_for(&pool, t2.id).await.unwrap();
        assert!(t2_last.is_none(), "fetch_error exclu du dédup");
    }

    #[tokio::test]
    async fn get_snapshot_returns_none_on_missing_id() {
        let (pool, _tmp) = test_pool().await;
        let got = get_snapshot(&pool, 99999).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn delete_target_cascades_snapshots() {
        let (pool, _tmp) = test_pool().await;
        let t = add_target(&pool, "https://c.example").await.unwrap();
        insert_snapshot(
            &pool,
            &NewSnapshot {
                target_id: t.id,
                url: "https://c.example",
                taken_at: "2026-04-19T10:00:00Z".into(),
                size_bytes: 5,
                sha256: "deadbeef",
                local_path: "/tmp/x.html",
                drive_file_id: None,
                status: "ok",
                error: None,
            },
        )
        .await
        .unwrap();

        delete_target(&pool, t.id).await.unwrap();
        let remaining = list_snapshots(&pool, Some(t.id), 100).await.unwrap();
        assert!(
            remaining.is_empty(),
            "ON DELETE CASCADE doit purger les snapshots (FK activées via db::open)"
        );
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        // Vérifie que le PRAGMA est bien positionné : insérer un snapshot
        // référençant une target inexistante doit échouer.
        let (pool, _tmp) = test_pool().await;
        let bad = insert_snapshot(
            &pool,
            &NewSnapshot {
                target_id: 424242,
                url: "https://ghost.example",
                taken_at: "2026-04-19T12:00:00Z".into(),
                size_bytes: 1,
                sha256: "00",
                local_path: "/tmp/ghost.html",
                drive_file_id: None,
                status: "ok",
                error: None,
            },
        )
        .await;
        assert!(
            bad.is_err(),
            "FK violation attendue sur target_id inexistant"
        );
    }
}
