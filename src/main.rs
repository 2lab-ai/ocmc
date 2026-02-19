use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    extract::{ws::WebSocketUpgrade, State},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, RwLock};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{error, info};

mod mc;

#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
    events_tx: broadcast::Sender<mc::McEvent>,
    cache: Arc<RwLock<mc::CacheState>>,
    cfg: mc::McConfig,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = mc::McConfig::from_env()?;
    let pool = SqlitePool::connect(&cfg.sqlite_url).await?;
    mc::db::migrate(&pool).await?;
    mc::db::ensure_admin(&pool, &cfg).await?;
    mc::db::seed_default_agents(&pool).await?;

    let (events_tx, _) = broadcast::channel(256);
    let cache = Arc::new(RwLock::new(mc::CacheState::default()));

    // Background poller: refresh snapshots + broadcast "refresh".
    {
        let pool = pool.clone();
        let events_tx = events_tx.clone();
        let cache = cache.clone();
        let cfg2 = cfg.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = mc::poller::tick(&pool, &cache, &cfg2).await {
                    error!("poller tick failed: {e:#}");
                } else {
                    let _ = events_tx.send(mc::McEvent::Refresh {
                        at: Utc::now(),
                        reason: "poll".to_string(),
                    });
                }
                tokio::time::sleep(Duration::from_millis(cfg2.poll_ms)).await;
            }
        });
    }

    let state = AppState {
        pool,
        events_tx,
        cache,
        cfg,
    };

    let addr: SocketAddr = format!("{}:{}", state.cfg.bind_host, state.cfg.bind_port)
        .parse()
        .context("invalid bind addr")?;

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/login", get(mc::auth_http::login_page).post(mc::auth_http::login_post))
        .route("/logout", post(mc::auth_http::logout_post))
        .route("/api/kanban", get(mc::handlers::kanban_get))
        .route("/api/task/:id/move", post(mc::handlers::task_move_post))
        .route("/api/task/:id/assign", post(mc::handlers::task_assign_post))
        .route("/api/cron/:id/toggle", post(mc::handlers::cron_toggle_post))
        .route("/api/cron/:id/run", post(mc::handlers::cron_run_post))
        .route("/ws", get(ws_handler))
        .nest_service("/", ServeDir::new("./static").append_index_html_on_directories(true))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!("mission-control listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| async move {
        mc::ws::serve(socket, state.events_tx.subscribe()).await;
    })
}
