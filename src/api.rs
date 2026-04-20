use crate::db::{self, Settings};
use crate::drive;
use crate::worker::{self, WorkerHandle};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub worker: WorkerHandle,
    pub sa_path: PathBuf,
}

const MAX_SA_SIZE: usize = 128 * 1024;

pub fn router(state: AppState) -> Router {
    let index = include_str!("assets/index.html");
    let app_js = include_str!("assets/app.js");
    let styles = include_str!("assets/styles.css");

    Router::new()
        .route("/", get(move || async move { Html(index) }))
        .route(
            "/app.js",
            get(
                move || async move { ([(header::CONTENT_TYPE, "application/javascript")], app_js) },
            ),
        )
        .route(
            "/styles.css",
            get(move || async move { ([(header::CONTENT_TYPE, "text/css")], styles) }),
        )
        .route("/api/settings", get(get_settings).post(post_settings))
        .route("/api/targets", get(get_targets).post(post_target))
        .route("/api/targets/:id", delete(delete_target))
        .route("/api/targets/:id/toggle", post(toggle_target))
        .route("/api/snapshots", get(list_snapshots))
        .route("/api/snapshots/:id", get(get_snapshot_meta))
        .route("/api/snapshots/:id/raw", get(snapshot_raw))
        .route("/api/snapshots/:id/view", get(snapshot_view))
        .route(
            "/api/drive/service-account",
            get(drive_sa_status)
                .post(drive_sa_upload)
                .delete(drive_sa_delete),
        )
        .route("/api/drive/test", post(drive_test))
        .route("/api/trigger", post(trigger))
        .route("/api/health", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_SA_SIZE))
        .with_state(Arc::new(state))
}

type S = State<Arc<AppState>>;

// --- settings ---

async fn get_settings(State(s): S) -> Result<Json<Settings>, ApiError> {
    Ok(Json(db::load_settings(&s.pool).await?))
}

async fn post_settings(
    State(s): S,
    Json(body): Json<Settings>,
) -> Result<Json<Settings>, ApiError> {
    db::save_settings(&s.pool, &body).await?;
    s.worker.trigger.notify_one();
    Ok(Json(body))
}

// --- targets ---

async fn get_targets(State(s): S) -> Result<Json<Vec<db::Target>>, ApiError> {
    Ok(Json(db::list_targets(&s.pool).await?))
}

#[derive(Deserialize)]
struct NewTargetReq {
    url: String,
}

async fn post_target(
    State(s): S,
    Json(body): Json<NewTargetReq>,
) -> Result<Json<db::Target>, ApiError> {
    let url = body.url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ApiError::bad_request("URL must start with http(s)://"));
    }
    Ok(Json(db::add_target(&s.pool, url).await?))
}

async fn delete_target(State(s): S, Path(id): Path<i64>) -> Result<(), ApiError> {
    db::delete_target(&s.pool, id).await?;
    Ok(())
}

#[derive(Deserialize)]
struct ToggleReq {
    enabled: bool,
}

async fn toggle_target(
    State(s): S,
    Path(id): Path<i64>,
    Json(body): Json<ToggleReq>,
) -> Result<(), ApiError> {
    db::set_target_enabled(&s.pool, id, body.enabled).await?;
    Ok(())
}

// --- snapshots ---

#[derive(Deserialize)]
struct ListQ {
    target_id: Option<i64>,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    100
}

async fn list_snapshots(
    State(s): S,
    Query(q): Query<ListQ>,
) -> Result<Json<Vec<db::Snapshot>>, ApiError> {
    Ok(Json(
        db::list_snapshots(&s.pool, q.target_id, q.limit).await?,
    ))
}

async fn get_snapshot_meta(
    State(s): S,
    Path(id): Path<i64>,
) -> Result<Json<db::Snapshot>, ApiError> {
    db::get_snapshot(&s.pool, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("snapshot"))
}

async fn snapshot_raw(State(s): S, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let snap = db::get_snapshot(&s.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("snapshot"))?;
    let bytes = worker::read_local(std::path::Path::new(&snap.local_path))?;
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], bytes).into_response())
}

async fn snapshot_view(State(s): S, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let snap = db::get_snapshot(&s.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("snapshot"))?;
    let bytes = worker::read_local(std::path::Path::new(&snap.local_path))?;
    let mut html = String::from_utf8_lossy(&bytes).into_owned();
    if let Some(pos) = html.to_lowercase().find("<head>") {
        if !html.to_lowercase().contains("<base") {
            let tag = format!("<base href=\"{}\">", snap.url);
            html.insert_str(pos + "<head>".len(), &tag);
        }
    }
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response())
}

// --- drive service account ---

#[derive(Serialize)]
struct DriveSaStatus {
    present: bool,
    client_email: Option<String>,
    project_id: Option<String>,
}

async fn drive_sa_status(State(s): S) -> Result<Json<DriveSaStatus>, ApiError> {
    if !s.sa_path.exists() {
        return Ok(Json(DriveSaStatus {
            present: false,
            client_email: None,
            project_id: None,
        }));
    }
    let raw = tokio::fs::read(&s.sa_path)
        .await
        .map_err(|e| ApiError::internal(format!("read sa: {e}")))?;
    let v: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|_| ApiError::bad_request("stored SA file is not valid JSON"))?;
    Ok(Json(DriveSaStatus {
        present: true,
        client_email: v
            .get("client_email")
            .and_then(|x| x.as_str())
            .map(String::from),
        project_id: v
            .get("project_id")
            .and_then(|x| x.as_str())
            .map(String::from),
    }))
}

async fn drive_sa_upload(State(s): S, body: Bytes) -> Result<Json<DriveSaStatus>, ApiError> {
    if body.is_empty() {
        return Err(ApiError::bad_request("empty body"));
    }
    if body.len() > MAX_SA_SIZE {
        return Err(ApiError::bad_request("file too large"));
    }

    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid JSON"))?;

    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if ty != "service_account" {
        return Err(ApiError::bad_request(
            "not a service_account key (check the 'type' field)",
        ));
    }
    for k in ["client_email", "private_key", "token_uri", "project_id"] {
        if v.get(k).and_then(|x| x.as_str()).unwrap_or("").is_empty() {
            return Err(ApiError::bad_request(format!("missing field: {k}")));
        }
    }

    let parent = s
        .sa_path
        .parent()
        .ok_or_else(|| ApiError::bad_request("bad sa path"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| ApiError::internal(format!("mkdir: {e}")))?;
    let tmp = parent.join(".sa.upload.tmp");
    tokio::fs::write(&tmp, &body)
        .await
        .map_err(|e| ApiError::internal(format!("write tmp: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&tmp)
            .await
            .map_err(|e| ApiError::internal(format!("stat tmp: {e}")))?
            .permissions();
        perms.set_mode(0o600);
        tokio::fs::set_permissions(&tmp, perms)
            .await
            .map_err(|e| ApiError::internal(format!("chmod tmp: {e}")))?;
    }

    tokio::fs::rename(&tmp, &s.sa_path)
        .await
        .map_err(|e| ApiError::internal(format!("rename: {e}")))?;

    s.worker.trigger.notify_one();

    Ok(Json(DriveSaStatus {
        present: true,
        client_email: v
            .get("client_email")
            .and_then(|x| x.as_str())
            .map(String::from),
        project_id: v
            .get("project_id")
            .and_then(|x| x.as_str())
            .map(String::from),
    }))
}

async fn drive_sa_delete(State(s): S) -> Result<(), ApiError> {
    if s.sa_path.exists() {
        tokio::fs::remove_file(&s.sa_path)
            .await
            .map_err(|e| ApiError::internal(format!("rm: {e}")))?;
    }
    Ok(())
}

/// Fait un upload dummy pour valider la config complète (SA + folder_id +
/// partage), puis supprime le fichier test côté Drive.
async fn drive_test(State(s): S) -> Result<Json<serde_json::Value>, ApiError> {
    let settings = db::load_settings(&s.pool).await?;
    if settings.drive_folder_id.trim().is_empty() {
        return Err(ApiError::bad_request("drive_folder_id is empty"));
    }
    if !s.sa_path.exists() {
        return Err(ApiError::bad_request("no service account uploaded"));
    }

    let token = drive::get_token(&s.sa_path).await?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let fname = format!(
        "torsnap_test_{}.html",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    let body = b"<!doctype html><title>torsnap test</title>";
    let file_id = drive::upload(&http, &token, &settings.drive_folder_id, &fname, body).await?;

    // Cleanup best-effort
    let _ = http
        .delete(format!(
            "https://www.googleapis.com/drive/v3/files/{file_id}"
        ))
        .bearer_auth(&token)
        .send()
        .await;

    Ok(Json(serde_json::json!({
        "ok": true,
        "uploaded_file_id": file_id,
        "folder_id": settings.drive_folder_id,
    })))
}

async fn trigger(State(s): S) -> &'static str {
    s.worker.trigger.notify_one();
    "triggered"
}

// --- errors ---

pub struct ApiError {
    status: StatusCode,
    msg: String,
}

impl ApiError {
    fn bad_request(m: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            msg: m.into(),
        }
    }
    fn not_found(m: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            msg: m.into(),
        }
    }
    fn internal(m: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg: m.into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg: e.to_string(),
        }
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg: e.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.msg }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::worker::WorkerHandle;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    async fn mk_state() -> (Router, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("t.db");
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let pool = db::open(&url).await.unwrap();
        let state = AppState {
            pool,
            worker: WorkerHandle {
                trigger: Arc::new(Notify::new()),
            },
            sa_path: dir.path().join("service_account.json"),
        };
        (router(state), dir)
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let (app, _tmp) = mk_state().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, b"ok");
    }

    #[tokio::test]
    async fn post_target_rejects_missing_scheme() {
        let (app, _tmp) = mk_state().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/targets")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":"example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let v: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert!(
            v["error"].as_str().unwrap_or("").contains("http"),
            "message d'erreur attendu sur le scheme, reçu {v}"
        );
    }

    #[tokio::test]
    async fn post_target_accepts_https_and_persists() {
        let (app, _tmp) = mk_state().await;
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/targets")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":"https://example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let created: db::Target = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(created.url, "https://example.com");

        let list_resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/targets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_resp.status(), StatusCode::OK);
        let listed: Vec<db::Target> = serde_json::from_slice(&body_bytes(list_resp).await).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].url, "https://example.com");
    }

    #[tokio::test]
    async fn drive_sa_status_reports_absent_when_no_file() {
        let (app, _tmp) = mk_state().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/drive/service-account")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(v["present"], serde_json::Value::Bool(false));
        assert!(v["client_email"].is_null());
    }

    #[tokio::test]
    async fn drive_sa_upload_rejects_non_service_account() {
        let (app, _tmp) = mk_state().await;
        let body = r#"{"type":"other","client_email":"x","private_key":"y","token_uri":"z","project_id":"p"}"#;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/drive/service-account")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn drive_sa_upload_rejects_empty_body() {
        let (app, _tmp) = mk_state().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/drive/service-account")
                    .header("content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn trigger_notifies_worker_and_returns_200() {
        let (app, _tmp) = mk_state().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/trigger")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, b"triggered");
    }
}
