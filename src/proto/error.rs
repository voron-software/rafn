use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid benchmark data: {0}")]
    InvalidBenchmark(String),
}

pub type Result<T> = std::result::Result<T, Error>;
