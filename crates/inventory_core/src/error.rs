use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// A user-facing message: bad input, not enough stock, etc.
    #[error("{0}")]
    Validation(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("Excel error: {0}")]
    Excel(String),
    #[error("Sync error: {0}")]
    Sync(String),
    #[error("File error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Data error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<calamine::Error> for Error {
    fn from(e: calamine::Error) -> Self {
        Error::Excel(e.to_string())
    }
}

impl From<rust_xlsxwriter::XlsxError> for Error {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        Error::Excel(e.to_string())
    }
}

#[cfg(feature = "client")]
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() || e.is_timeout() {
            Error::Sync(
                "Cannot reach the desktop. Check that the PC app is open and both devices are on the same Wi-Fi.".into(),
            )
        } else {
            Error::Sync(e.to_string())
        }
    }
}

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Validation(msg.into()))
}
