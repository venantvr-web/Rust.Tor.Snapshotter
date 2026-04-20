use crate::db::{self, NewSnapshot, Settings};
use crate::drive;
use anyhow::Result;
use chrono::Utc;
use reqwest::Client;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct WorkerHandle {
    /// Réveille le worker pour un tick immédiat (bouton "Run now").
    pub trigger: Arc<Notify>,
}

pub struct WorkerCtx {
    pub pool: SqlitePool,
    pub cache_dir: PathBuf,
    pub service_account: Option<PathBuf>,
}

pub fn spawn(ctx: WorkerCtx) -> WorkerHandle {
    let trigger = Arc::new(Notify::new());
    let trigger_clone = trigger.clone();

    tokio::spawn(async move {
        run(ctx, trigger_clone).await;
    });

    WorkerHandle { trigger }
}

async fn run(ctx: WorkerCtx, trigger: Arc<Notify>) {
    loop {
        // Recharge la config à chaque tick : les changements de l'UI
        // sont pris en compte sans redémarrage.
        let settings = match db::load_settings(&ctx.pool).await {
            Ok(s) => s,
            Err(e) => {
                error!("chargement settings: {e:#}");
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        if let Err(e) = tick(&ctx, &settings).await {
            error!("tick échoué: {e:#}");
        }

        let delay = Duration::from_secs(settings.interval_secs.max(10) as u64);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = trigger.notified() => {
                info!("tick déclenché manuellement");
            }
        }
    }
}

async fn tick(ctx: &WorkerCtx, s: &Settings) -> Result<()> {
    let targets = db::list_enabled_targets(&ctx.pool).await?;
    if targets.is_empty() {
        info!("aucune cible active");
        return Ok(());
    }

    let tor = build_tor_client(s)?;
    let direct = Client::builder()
        .timeout(Duration::from_secs(s.http_timeout_secs as u64))
        .build()?;

    // Token Drive : récupéré 1× par tick si l'upload est activé.
    let drive_token = if s.drive_enabled {
        match &ctx.service_account {
            Some(p) if p.exists() => match drive::get_token(p).await {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!("token Drive indisponible: {e:#}");
                    None
                }
            },
            _ => {
                warn!("drive_enabled mais service_account absent");
                None
            }
        }
    } else {
        None
    };

    for t in targets {
        if let Err(e) = snapshot_one(ctx, s, &tor, &direct, drive_token.as_deref(), &t).await {
            error!(url = %t.url, "snapshot: {e:#}");
        }
    }
    Ok(())
}

async fn snapshot_one(
    ctx: &WorkerCtx,
    s: &Settings,
    tor: &Client,
    direct: &Client,
    drive_token: Option<&str>,
    t: &db::Target,
) -> Result<()> {
    let taken_at = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let body = match tor
        .get(&t.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => match r.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
                db::insert_snapshot(
                    &ctx.pool,
                    &NewSnapshot {
                        target_id: t.id,
                        url: &t.url,
                        taken_at,
                        size_bytes: 0,
                        sha256: "",
                        local_path: "",
                        drive_file_id: None,
                        status: "fetch_error",
                        error: Some(&e.to_string()),
                    },
                )
                .await?;
                return Ok(());
            }
        },
        Err(e) => {
            db::insert_snapshot(
                &ctx.pool,
                &NewSnapshot {
                    target_id: t.id,
                    url: &t.url,
                    taken_at,
                    size_bytes: 0,
                    sha256: "",
                    local_path: "",
                    drive_file_id: None,
                    status: "fetch_error",
                    error: Some(&e.to_string()),
                },
            )
            .await?;
            return Ok(());
        }
    };

    let sha = {
        let mut h = Sha256::new();
        h.update(&body);
        hex::encode(h.finalize())
    };

    // Dédup : si le contenu est strictement identique au dernier snapshot OK,
    // on ne re-stocke pas. On log juste.
    if let Some(last) = db::last_sha_for(&ctx.pool, t.id).await? {
        if last == sha {
            info!(url = %t.url, "contenu inchangé (sha identique), skip");
            return Ok(());
        }
    }

    // Sauvegarde locale
    let host = host_of(&t.url);
    let dir = ctx.cache_dir.join(&host);
    tokio::fs::create_dir_all(&dir).await?;
    let fname = format!("{}_{}.html", taken_at.replace(':', ""), &sha[..12]);
    let path = dir.join(&fname);
    tokio::fs::write(&path, &body).await?;

    // Upload Drive si activé
    let mut drive_id = None;
    let mut status = "ok";
    let mut err: Option<String> = None;
    if let Some(tok) = drive_token {
        match drive::upload(direct, tok, &s.drive_folder_id, &fname, &body).await {
            Ok(id) => drive_id = Some(id),
            Err(e) => {
                warn!(url = %t.url, "upload Drive: {e:#}");
                status = "upload_error";
                err = Some(e.to_string());
            }
        }
    }

    db::insert_snapshot(
        &ctx.pool,
        &NewSnapshot {
            target_id: t.id,
            url: &t.url,
            taken_at,
            size_bytes: body.len() as i64,
            sha256: &sha,
            local_path: &path.to_string_lossy(),
            drive_file_id: drive_id.as_deref(),
            status,
            error: err.as_deref(),
        },
    )
    .await?;

    info!(url = %t.url, size = body.len(), "snapshot ok");
    Ok(())
}

fn build_tor_client(s: &Settings) -> Result<Client> {
    let proxy = reqwest::Proxy::all(&s.tor_socks)?;
    Ok(Client::builder()
        .proxy(proxy)
        .user_agent(&s.user_agent)
        .timeout(Duration::from_secs(s.http_timeout_secs as u64))
        .build()?)
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

pub fn read_local(path: &Path) -> Result<Vec<u8>> {
    Ok(std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_strips_scheme_and_path() {
        assert_eq!(host_of("https://example.com/foo/bar"), "example.com");
        assert_eq!(host_of("http://example.com"), "example.com");
        assert_eq!(
            host_of("https://check.torproject.org/?lang=fr"),
            "check.torproject.org"
        );
    }

    #[test]
    fn host_of_preserves_port() {
        assert_eq!(host_of("http://localhost:8080/path"), "localhost:8080");
    }

    #[test]
    fn host_of_onion_address() {
        assert_eq!(
            host_of("http://duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion/"),
            "duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion"
        );
    }

    #[test]
    fn host_of_without_scheme_returns_leading_segment() {
        // Cas dégradé : l'URL n'a pas de scheme → on retombe sur la 1re
        // portion avant le `/`. La validation côté API empêche normalement
        // cette situation, mais la fonction doit rester sûre.
        assert_eq!(host_of("example.com/x"), "example.com");
    }

    #[test]
    fn read_local_roundtrips_bytes() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("snap.html");
        let payload = b"<!doctype html><title>t</title>";
        std::fs::write(&p, payload).unwrap();
        let got = read_local(&p).unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn read_local_errors_on_missing_path() {
        let err = read_local(Path::new("/nonexistent/does_not_exist_xyz.html"));
        assert!(err.is_err());
    }
}
