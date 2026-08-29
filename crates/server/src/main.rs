mod admin;
mod auth;
mod error;
mod public;
mod state;
mod views;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, header},
    response::Redirect,
    routing::{get, post},
};
use domain::{auth::AuthService, diary::DiaryService, images::ImageService};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer, trace::TraceLayer};

use crate::state::AppState;

/// Photos come straight off a phone camera, which the 2 MB default would
/// reject. The limit is per request, and an entry can carry several.
const MAX_UPLOAD: usize = 64 * 1024 * 1024;

/// Everything behind the admin session. The `Admin` argument on each handler
/// is what enforces that -- see the extractor in `auth.rs`.
fn admin_routes() -> Router<AppState> {
    return Router::new()
        .route("/admin", get(admin::index))
        .route("/admin/diaries", post(admin::create_diary))
        .route(
            "/admin/diaries/{id}",
            get(admin::show_diary).post(admin::update_diary),
        )
        .route("/admin/diaries/{id}/reroll", post(admin::reroll_diary))
        .route("/admin/diaries/{id}/delete", post(admin::delete_diary))
        .route(
            "/admin/diaries/{id}/entries",
            post(admin::create_entry).layer(DefaultBodyLimit::max(MAX_UPLOAD)),
        )
        .route(
            "/admin/photos",
            post(admin::upload_photo).layer(DefaultBodyLimit::max(MAX_UPLOAD)),
        )
        .route("/admin/entries/{id}", post(admin::update_entry))
        .route("/admin/entries/{id}/delete", post(admin::delete_entry))
        .route("/admin/photos/{id}/delete", post(admin::delete_photo));
}

/// Reachable without a session. The share token is the only credential.
fn public_routes() -> Router<AppState> {
    return Router::new()
        .route("/", get(|| async { Redirect::to("/admin") }))
        .route("/login", get(public::login_page).post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/d/{share_token}", get(public::shared_diary));
}

fn env_or(key: &str, default: &str) -> String {
    return std::env::var(key).unwrap_or_else(|_| default.to_string());
}

/// The uid the process ended up with, which is the other half of any
/// permission problem on a bind mount.
fn current_uid() -> String {
    #[cfg(unix)]
    {
        return unsafe { libc::getuid() }.to_string();
    }
    #[cfg(not(unix))]
    {
        return "n/a".to_string();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Before anything reads the environment, so RUST_LOG set in .env still
    // reaches the logger below. A missing file is the normal case in
    // production, where the values come from the container instead -- and
    // real environment variables always win over the file either way.
    match dotenvy::dotenv() {
        Ok(path) => println!("loaded environment from {}", path.display()),
        Err(err) if err.not_found() => {}
        // A malformed .env is worth failing on: the alternative is starting
        // with half the configuration silently missing.
        Err(err) => return Err(err.into()),
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "server=info,domain=info,tower_http=info".into()),
        )
        .init();

    // Random per deployment; there is no user database to check against.
    let admin_token =
        std::env::var("ADMIN_TOKEN").map_err(|_| anyhow::anyhow!("ADMIN_TOKEN must be set"))?;

    let data_dir = PathBuf::from(env_or("DATA_DIR", "data"));
    // Blobs can live on a different disk than the database, so they get their
    // own knob; without it they sit next to it.
    let images_dir = match std::env::var("FILE_STORAGE") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => data_dir.join("images"),
    };
    // Named explicitly: a bare "Permission denied (os error 13)" from a
    // container tells you nothing about which mount is wrong.
    for dir in [&data_dir, &images_dir] {
        std::fs::create_dir_all(dir).map_err(|err| {
            anyhow::anyhow!(
                "cannot create {} (running as uid {}): {err}",
                dir.display(),
                current_uid(),
            )
        })?;
    }

    let (write, read) = domain::db::connect(&data_dir.join("diary.db")).await?;

    let images = ImageService::new(images_dir.clone());
    let state = AppState {
        diaries: Arc::new(DiaryService::new(write, read, images.clone())),
        images,
        auth: Arc::new(AuthService::new(&admin_token)),
        secure_cookies: env_or("COOKIE_SECURE", "true") == "true",
    };

    // Content-addressed, so a given URL can never change what it returns.
    let blobs = Router::new()
        .fallback_service(ServeDir::new(&images_dir))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ));

    let app = Router::new()
        .merge(admin_routes())
        .merge(public_routes())
        .nest("/img", blobs)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // 0.0.0.0 because this runs in a container; publish it as
    // -p 127.0.0.1:3000:3000 so only the reverse proxy can reach it.
    let addr = env_or("BIND_ADDR", "0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");

    axum::serve(listener, app).await?;

    return Ok(());
}
