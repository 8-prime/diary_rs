use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// `domain::Error` and `IntoResponse` are both foreign to this crate, so a
/// domain error has to be wrapped before it can become a response. The other
/// two variants cover failures that never reach the domain at all -- a
/// malformed request body, or a template that would not render.
#[derive(Debug)]
pub enum AppError {
    Domain(domain::Error),
    BadRequest(String),
    Internal(String),
}

impl AppError {
    pub fn bad_request(err: impl std::fmt::Display) -> Self {
        return AppError::BadRequest(err.to_string());
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        return AppError::Internal(err.to_string());
    }
}

/// Lets handlers use `?` on anything that converts into a domain error.
impl<E> From<E> for AppError
where
    E: Into<domain::Error>,
{
    fn from(err: E) -> Self {
        return AppError::Domain(err.into());
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        use domain::Error::*;

        let status = match &self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Domain(err) => match err {
                NotFound => StatusCode::NOT_FOUND,
                Unauthorized | BadToken => StatusCode::UNAUTHORIZED,
                BadImage(_) | UnsupportedImage => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Time(_) => StatusCode::BAD_REQUEST,
                Db(_) | Migrate(_) | Resize(_) | Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            },
        };

        // The detail goes to the log, never to the client: a SQL error string
        // describes the schema, and the visitor cannot act on it anyway.
        match &self {
            AppError::Domain(err) if status.is_server_error() => {
                tracing::error!(error = %err, "request failed")
            }
            AppError::Internal(msg) => tracing::error!(error = %msg, "request failed"),
            AppError::BadRequest(msg) => tracing::debug!(error = %msg, "rejected request"),
            AppError::Domain(_) => {}
        }

        return (status, status.canonical_reason().unwrap_or("error")).into_response();
    }
}
