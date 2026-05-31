use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Unknown format")]
    UnknownFormat,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Proto error: {0}")]
    Proto(#[from] crate::proto::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
