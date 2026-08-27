use axum::{
    Form, Json,
    extract::{Multipart, Path, State},
    response::{Html, Redirect},
};
use domain::{
    auth::Admin,
    diary::{NewDiary, NewEntry, NewImage, timestamp_from_local},
};
use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    state::AppState,
    views::{AdminDiaryPage, AdminIndexPage, render, render_days},
};

/// Every handler here takes an `Admin`. That argument is the authorisation
/// check: the extractor in `auth.rs` runs before the body does, and the type
/// cannot be constructed any other way.
pub async fn index(_admin: Admin, State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let diaries = state.diaries.get_diaries().await?;

    return render(AdminIndexPage { diaries });
}

pub async fn show_diary(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, AppError> {
    let view = state.diaries.get_diary(id).await?;
    let days = render_days(&view, &state.images);

    return render(AdminDiaryPage {
        description: view.diary.description.clone().unwrap_or_default(),
        diary: view.diary,
        days,
    });
}

#[derive(Deserialize)]
pub struct DiaryForm {
    pub title: String,
    pub description: String,
    pub timezone: String,
}

pub async fn create_diary(
    _admin: Admin,
    State(state): State<AppState>,
    Form(form): Form<DiaryForm>,
) -> Result<Redirect, AppError> {
    let diary = state
        .diaries
        .create_diary(NewDiary {
            title: form.title,
            // An empty text input and "no description" are the same thing.
            description: blank_to_none(form.description),
            timezone: form.timezone,
        })
        .await?;

    return Ok(Redirect::to(&format!("/admin/diaries/{}", diary.id)));
}

#[derive(Deserialize)]
pub struct DiaryUpdateForm {
    pub title: String,
    pub description: String,
}

pub async fn update_diary(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<DiaryUpdateForm>,
) -> Result<Redirect, AppError> {
    let description = blank_to_none(form.description);

    state
        .diaries
        .update_diary(id, &form.title, description.as_deref())
        .await?;

    return Ok(Redirect::to(&format!("/admin/diaries/{id}")));
}

pub async fn reroll_diary(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Redirect, AppError> {
    state.diaries.reroll_diary_keys(id).await?;

    return Ok(Redirect::to(&format!("/admin/diaries/{id}")));
}

pub async fn delete_diary(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Redirect, AppError> {
    state.diaries.delete_diary(id).await?;

    return Ok(Redirect::to("/admin"));
}

#[derive(Serialize)]
pub struct UploadedPhoto {
    pub hash: String,
    pub width: u32,
    pub height: u32,
}

/// Stores one blob and reports what it became. The page posts here as soon as
/// a file is picked, so the slow part is over before the entry is submitted --
/// nothing is written to the database, and an abandoned blob is reclaimed by
/// the sweep.
pub async fn upload_photo(
    _admin: Admin,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadedPhoto>, AppError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(AppError::bad_request)?
    {
        if field.name().unwrap_or_default() != "photo" {
            continue;
        }

        let bytes = field.bytes().await.map_err(AppError::bad_request)?;
        let uploaded = state.images.upload_image(bytes.to_vec()).await?;

        return Ok(Json(UploadedPhoto {
            hash: uploaded.hash,
            width: uploaded.width,
            height: uploaded.height,
        }));
    }

    return Err(AppError::BadRequest("no photo field".into()));
}

/// Text and photos arrive together in one multipart POST, so posting an entry
/// works without any JavaScript. The blobs are written first and the row is
/// written second -- the other order would reference a file that is not there
/// yet, and a blob with no row is only wasted disk until the sweep runs.
pub async fn create_entry(
    _admin: Admin,
    State(state): State<AppState>,
    Path(diary_id): Path<i64>,
    mut multipart: Multipart,
) -> Result<Redirect, AppError> {
    let mut text = String::new();
    let mut occurred_at_local = String::new();
    let mut images: Vec<NewImage> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(AppError::bad_request)?
    {
        match field.name().unwrap_or_default() {
            "text" => text = field.text().await.map_err(AppError::bad_request)?,
            "occurred_at" => {
                occurred_at_local = field.text().await.map_err(AppError::bad_request)?
            }
            // Sent by the page once a photo has already been uploaded. The
            // three fields arrive in order, so each hash opens a new image
            // and the dimensions that follow belong to it.
            "hash" => {
                let hash = field.text().await.map_err(AppError::bad_request)?;
                images.push(NewImage {
                    hash,
                    width: 0,
                    height: 0,
                    alt: None,
                });
            }
            "width" | "height" => {
                let name = field.name().unwrap_or_default().to_string();
                let value: u32 = field
                    .text()
                    .await
                    .map_err(AppError::bad_request)?
                    .parse()
                    .map_err(AppError::bad_request)?;

                let Some(image) = images.last_mut() else {
                    return Err(AppError::BadRequest(format!("{name} before hash")));
                };

                if name == "width" {
                    image.width = value;
                } else {
                    image.height = value;
                }
            }
            // The no-JavaScript fallback: the bytes themselves came along.
            "photos" => {
                let bytes = field.bytes().await.map_err(AppError::bad_request)?;
                // An empty file input still sends one empty part.
                if bytes.is_empty() {
                    continue;
                }

                let uploaded = state.images.upload_image(bytes.to_vec()).await?;
                images.push(NewImage {
                    hash: uploaded.hash,
                    width: uploaded.width,
                    height: uploaded.height,
                    alt: None,
                });
            }
            _ => {}
        }
    }

    let occurred_at = if occurred_at_local.is_empty() {
        now_unix()
    } else {
        let timezone = state.diaries.diary_timezone(diary_id).await?;
        timestamp_from_local(&timezone, &occurred_at_local)?
    };

    state
        .diaries
        .create_entry(
            diary_id,
            NewEntry {
                occurred_at,
                text: blank_to_none(text),
                images,
            },
        )
        .await?;

    return Ok(Redirect::to(&format!("/admin/diaries/{diary_id}")));
}

#[derive(Deserialize)]
pub struct EntryForm {
    pub text: String,
}

pub async fn update_entry(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<EntryForm>,
) -> Result<Redirect, AppError> {
    let text = blank_to_none(form.text);
    let entry = state.diaries.update_entry(id, text.as_deref()).await?;

    return Ok(Redirect::to(&format!(
        "/admin/diaries/{}",
        entry.diary_id
    )));
}

pub async fn delete_entry(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Redirect, AppError> {
    // Read the parent before the row goes away, so we know where to go back to.
    let entry = state.diaries.get_entry(id).await?;
    state.diaries.delete_entry(id).await?;

    return Ok(Redirect::to(&format!(
        "/admin/diaries/{}",
        entry.diary_id
    )));
}

pub async fn delete_photo(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Redirect, AppError> {
    let entry_id = state.diaries.delete_photo(id).await?;
    let entry = state.diaries.get_entry(entry_id).await?;

    return Ok(Redirect::to(&format!(
        "/admin/diaries/{}",
        entry.diary_id
    )));
}

fn now_unix() -> i64 {
    return std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default();
}

fn blank_to_none(value: String) -> Option<String> {
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return None;
    }

    return Some(trimmed.to_string());
}
