use std::sync::Arc;

use domain::{auth::AuthService, diary::DiaryService, images::ImageService};

/// Axum clones the state for every request, so the expensive parts sit behind
/// an `Arc`. `ImageService` is just a path, so it is cloned directly.
#[derive(Clone)]
pub struct AppState {
    pub diaries: Arc<DiaryService>,
    pub images: ImageService,
    pub auth: Arc<AuthService>,
    /// Off only for local development over plain http, where a `Secure` cookie
    /// would be set and then never sent back.
    pub secure_cookies: bool,
}
