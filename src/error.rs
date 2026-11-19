use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("too large: {0}")]
    TooLarge(String),
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("file count limit exceeded")]
    EntryLimitExceeded,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("permission denied")]
    PermissionDenied,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("confirmation required")]
    ConfirmRequired,
    #[error("precondition failed")]
    PreconditionFailed,
    #[error("unsupported media: {0}")]
    UnsupportedMedia(String),
    #[error("setup required")]
    SetupRequired,
    #[error("too many requests")]
    TooManyRequests,
    #[error("background queue is full")]
    QueueFull,
    #[error("operation cancelled")]
    Cancelled,
    #[error("not found")]
    NotFound,
    #[error("share has been stopped")]
    ShareRevoked,
    #[error("share has expired")]
    ShareExpired,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn invalid_path(message: impl Into<String>) -> Self {
        Self::InvalidPath(message.into())
    }

    pub fn too_large(message: impl Into<String>) -> Self {
        Self::TooLarge(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn internal(err: impl Into<anyhow::Error>) -> Self {
        Self::Internal(err.into())
    }

    /// Stable machine-readable code shared by HTTP errors and job failures.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "BAD_REQUEST",
            Self::InvalidPath(_) => "INVALID_PATH",
            Self::TooLarge(_) => "TOO_LARGE",
            Self::QuotaExceeded => "QUOTA_EXCEEDED",
            Self::EntryLimitExceeded => "ENTRY_LIMIT_EXCEEDED",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::Conflict(_) => "CONFLICT",
            Self::ConfirmRequired => "CONFIRM_REQUIRED",
            Self::PreconditionFailed => "PRECONDITION_FAILED",
            Self::UnsupportedMedia(_) => "UNSUPPORTED_MEDIA",
            Self::SetupRequired => "SETUP_REQUIRED",
            Self::TooManyRequests => "RATE_LIMITED",
            Self::QueueFull => "QUEUE_FULL",
            Self::Cancelled => "CANCELLED",
            Self::NotFound => "NOT_FOUND",
            Self::ShareRevoked => "SHARE_REVOKED",
            Self::ShareExpired => "SHARE_EXPIRED",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", message.clone()),
            Self::InvalidPath(message) => {
                (StatusCode::BAD_REQUEST, "INVALID_PATH", message.clone())
            }
            Self::TooLarge(message) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "TOO_LARGE", message.clone())
            }
            Self::QuotaExceeded => (
                StatusCode::INSUFFICIENT_STORAGE,
                "QUOTA_EXCEEDED",
                "User quota exceeded".to_owned(),
            ),
            Self::EntryLimitExceeded => (
                StatusCode::INSUFFICIENT_STORAGE,
                "ENTRY_LIMIT_EXCEEDED",
                "File count limit exceeded".to_owned(),
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Authentication required".to_owned(),
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN", "Forbidden".to_owned()),
            Self::PermissionDenied => (
                StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "The server process does not have permission to access this path".to_owned(),
            ),
            Self::Conflict(message) => (StatusCode::CONFLICT, "CONFLICT", message.clone()),
            Self::ConfirmRequired => (
                StatusCode::CONFLICT,
                "CONFIRM_REQUIRED",
                "Confirmation is required".to_owned(),
            ),
            Self::PreconditionFailed => (
                StatusCode::PRECONDITION_FAILED,
                "PRECONDITION_FAILED",
                "The file has changed".to_owned(),
            ),
            Self::UnsupportedMedia(message) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "UNSUPPORTED_MEDIA",
                message.clone(),
            ),
            Self::SetupRequired => (
                StatusCode::CONFLICT,
                "SETUP_REQUIRED",
                "Initial setup is required".to_owned(),
            ),
            Self::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMITED",
                "Too many attempts, try again later".to_owned(),
            ),
            Self::QueueFull => (
                StatusCode::SERVICE_UNAVAILABLE,
                "QUEUE_FULL",
                "Too many background jobs are pending; try again later".to_owned(),
            ),
            Self::Cancelled => (
                StatusCode::CONFLICT,
                "CANCELLED",
                "The operation was cancelled".to_owned(),
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", "Not found".to_owned()),
            Self::ShareRevoked => (
                StatusCode::GONE,
                "SHARE_REVOKED",
                "Share has been stopped".to_owned(),
            ),
            Self::ShareExpired => (
                StatusCode::GONE,
                "SHARE_EXPIRED",
                "Share has expired".to_owned(),
            ),
            Self::Internal(err) => {
                tracing::error!(error = %err, "internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "Internal server error".to_owned(),
                )
            }
        };

        (
            status,
            Json(json!({
                "error": {
                    "code": code,
                    "message": message,
                }
            })),
        )
            .into_response()
    }
}
