mod api;
mod db;
mod drive;
mod worker;

use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rust_tor_snapshotter=debug,sqlx=warn".into()),
        )
        .init();

    let data_dir: PathBuf = std::env::var("DATA_DIR")
        .unwrap_or_else(|_| "./data".into())
        .into();
    let cache_dir: PathBuf = std::env::var("CACHE_DIR")
        .unwrap_or_else(|_| data_dir.join("snapshots").to_string_lossy().into_owned())
        .into();
    let sa_path: PathBuf = std::env::var("GOOGLE_SERVICE_ACCOUNT")
        .unwrap_or_else(|_| {
            data_dir
                .join("service_account.json")
                .to_string_lossy()
                .into()
        })
        .into();
    // BIND_ADDR prime sur PORT ; si aucun des deux n'est défini, on écoute
    // sur 0.0.0.0:8080 pour conserver le comportement historique.
    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| {
        let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
        format!("0.0.0.0:{port}")
    });

    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&cache_dir)?;

    let db_path = data_dir.join("state.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = db::open(&db_url).await?;

    let handle = worker::spawn(worker::WorkerCtx {
        pool: pool.clone(),
        cache_dir: cache_dir.clone(),
        service_account: Some(sa_path.clone()),
    });

    let state = api::AppState {
        pool,
        worker: handle,
        sa_path: sa_path.clone(),
    };
    let app = api::router(state).layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    info!("écoute sur http://{bind}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("arrêt demandé");
}
