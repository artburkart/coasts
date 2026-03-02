use std::sync::Arc;

use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use coast_core::protocol::{UpdateApplyResponse, UpdateCheckResponse};

use crate::server::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/update/check", get(check_update))
        .route("/update/apply", post(apply_update))
}

async fn check_update() -> Json<UpdateCheckResponse> {
    let info = coast_update::check_for_updates().await;
    let update_available = info
        .latest_version
        .as_ref()
        .and_then(|latest| {
            let current = coast_update::version::parse_version(&info.current_version).ok()?;
            let latest = coast_update::version::parse_version(latest).ok()?;
            Some(coast_update::version::is_newer(&current, &latest))
        })
        .unwrap_or(false);

    Json(UpdateCheckResponse {
        current_version: info.current_version,
        latest_version: info.latest_version,
        update_available,
    })
}

async fn apply_update() -> Result<Json<UpdateApplyResponse>, (StatusCode, Json<serde_json::Value>)>
{
    let info = coast_update::check_for_updates().await;
    let Some(latest_str) = info.latest_version else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Could not determine latest version" })),
        ));
    };

    let latest_ver = coast_update::version::parse_version(&latest_str).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Invalid version: {e}") })),
        )
    })?;

    let tarball =
        coast_update::updater::download_release(&latest_ver, coast_update::DOWNLOAD_TIMEOUT)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Download failed: {e}") })),
                )
            })?;

    coast_update::updater::apply_update(&tarball).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Apply failed: {e}") })),
        )
    })?;

    let version = latest_str.clone();

    // Schedule a self-restart so the new binary takes over.
    // The 500ms delay lets the HTTP response reach the client first.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        restart_daemon();
    });

    Ok(Json(UpdateApplyResponse {
        success: true,
        version,
    }))
}

/// Replace the current process with the new daemon binary.
fn restart_daemon() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("cannot determine current exe for restart: {e}");
            return;
        }
    };

    let args: Vec<String> = std::env::args().collect();

    tracing::info!("restarting daemon: {}", exe.display());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // exec() replaces the process — this never returns on success
        let err = std::process::Command::new(&exe).args(&args[1..]).exec();
        tracing::error!("exec failed: {err}");
    }

    #[cfg(not(unix))]
    {
        // Fallback: spawn new process and exit
        let _ = std::process::Command::new(&exe).args(&args[1..]).spawn();
        std::process::exit(0);
    }
}
