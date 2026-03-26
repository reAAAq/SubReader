//! Core parser module for EPUB and TXT format parsing.
//!
//! This crate provides format-agnostic parsing of book files into
//! a platform-independent DOM tree representation.

pub mod epub;
pub mod error;
pub mod txt;

pub use epub::EpubParser;
pub use error::ParseError;
pub use txt::TxtParser;
pub use txt::{TxtChapter, TxtParseResult};
