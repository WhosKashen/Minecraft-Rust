use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("network error: {0}")]
    Net(String),
    #[error("{0}")]
    Invalid(String),
    #[error("checksum mismatch for {file}: expected {expected}, got {actual}")]
    Checksum { file: String, expected: String, actual: String },
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("cancelled")]
    Cancelled,
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        // Strip the URL query (could carry identifiers) from the message.
        Error::Net(e.without_url().to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}
