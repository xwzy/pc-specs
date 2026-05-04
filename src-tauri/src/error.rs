use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("collect failed: {0}")]
    Collect(String),

    #[error("not supported on this platform")]
    Unsupported,

    #[error("{0}")]
    Other(String),
}

#[allow(dead_code)]
pub type AppResult<T> = std::result::Result<T, AppError>;

impl AppError {
    pub fn other<S: Into<String>>(s: S) -> Self {
        AppError::Other(s.into())
    }
}

impl From<AppError> for String {
    fn from(value: AppError) -> Self {
        value.to_string()
    }
}
