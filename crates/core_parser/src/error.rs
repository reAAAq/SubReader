//! Parser error types.

use thiserror::Error;

/// Errors that can occur during book parsing.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Invalid EPUB file: {0}")]
    InvalidEpub(String),

    #[error("Invalid ZIP archive: {0}")]
    InvalidZip(String),

    #[error("Invalid XML content: {0}")]
    InvalidXml(String),

    #[error("DRM-protected content detected")]
    DrmProtected,

    #[error("I/O error: {0}")]
    IoError(String),

    #[error("Unsupported encoding: {0}")]
    UnsupportedEncoding(String),

    #[error("File too large: {size} bytes (max: {max} bytes)")]
    FileTooLarge { size: u64, max: u64 },

    #[error("Empty content")]
    EmptyContent,
}

impl From<zip::result::ZipError> for ParseError {
    fn from(e: zip::result::ZipError) -> Self {
        ParseError::InvalidZip(e.to_string())
    }
}

impl From<quick_xml::Error> for ParseError {
    fn from(e: quick_xml::Error) -> Self {
        ParseError::InvalidXml(e.to_string())
    }
}
