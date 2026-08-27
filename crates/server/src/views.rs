use askama::Template;
use axum::response::Html;
use domain::{
    diary::{Diary, DiaryView},
    images::ImageService,
};

use crate::error::AppError;

pub fn render<T: Template>(template: T) -> Result<Html<String>, AppError> {
    return template.render().map(Html).map_err(AppError::internal);
}

/// An image with its srcset already built. This is where the two services meet
/// -- the rows come from the diary service, the URLs from the image service --
/// and doing it here keeps the templates free of any logic about widths.
pub struct RenderedImage {
    pub id: i64,
    pub src: String,
    pub srcset: String,
    pub width: i64,
    pub height: i64,
    pub alt: String,
}

pub struct RenderedEntry {
    pub id: i64,
    pub text: Option<String>,
    pub images: Vec<RenderedImage>,
}

pub struct RenderedDay {
    pub local_date: String,
    pub entries: Vec<RenderedEntry>,
}

pub fn render_days(view: &DiaryView, images: &ImageService) -> Vec<RenderedDay> {
    return view
        .days
        .iter()
        .map(|day| RenderedDay {
            local_date: day.local_date.clone(),
            entries: day
                .entries
                .iter()
                .map(|entry| RenderedEntry {
                    id: entry.entry.id,
                    text: entry.entry.text.clone(),
                    images: entry
                        .images
                        .iter()
                        .map(|image| {
                            let sources = images.derive_sourceset(&image.hash, image.width as u32);

                            RenderedImage {
                                id: image.id,
                                // The widest variant is the fallback for
                                // anything that ignores srcset.
                                src: sources
                                    .last()
                                    .map(|entry| entry.url.clone())
                                    .unwrap_or_default(),
                                srcset: sources
                                    .iter()
                                    .map(|entry| format!("{} {}w", entry.url, entry.width))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                width: image.width,
                                height: image.height,
                                // An empty alt is the correct markup for an
                                // image that carries no caption.
                                alt: image.alt.clone().unwrap_or_default(),
                            }
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub failed: bool,
}

#[derive(Template)]
#[template(path = "admin_index.html")]
pub struct AdminIndexPage {
    pub diaries: Vec<Diary>,
}

#[derive(Template)]
#[template(path = "admin_diary.html")]
pub struct AdminDiaryPage {
    pub diary: Diary,
    pub description: String,
    pub days: Vec<RenderedDay>,
}

#[derive(Template)]
#[template(path = "diary.html")]
pub struct DiaryPage {
    pub diary: Diary,
    pub days: Vec<RenderedDay>,
}
