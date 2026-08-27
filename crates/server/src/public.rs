use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::{
    error::AppError,
    state::AppState,
    views::{DiaryPage, LoginPage, render, render_days},
};

#[derive(Deserialize)]
pub struct LoginQuery {
    pub failed: Option<String>,
}

pub async fn login_page(Query(query): Query<LoginQuery>) -> Result<Html<String>, AppError> {
    return render(LoginPage {
        failed: query.failed.is_some(),
    });
}

/// The only public read path. The share token is the credential, so an unknown
/// one is a plain 404 -- the same answer a guess deserves.
pub async fn shared_diary(
    State(state): State<AppState>,
    Path(share_token): Path<String>,
) -> Result<Html<String>, AppError> {
    let view = state.diaries.get_shared_diary(&share_token).await?;
    let days = render_days(&view, &state.images);

    return render(DiaryPage {
        diary: view.diary,
        days,
    });
}
