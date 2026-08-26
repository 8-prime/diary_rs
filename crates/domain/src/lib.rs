pub mod auth;
pub mod diary;
pub mod images;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unauthorized")]
    Unauthorized,
    #[error("malformed session token")]
    BadToken,
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("unreadable image")]
    BadImage(#[from] image::ImageError),
    #[error("unsupported pixel format")]
    UnsupportedImage,
    #[error("resize failed")]
    Resize(#[from] fast_image_resize::ResizeError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
