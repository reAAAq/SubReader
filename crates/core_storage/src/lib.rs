//! Core storage module for SQLite local persistence.
//!
//! This crate provides the local database layer using SQLite (rusqlite).
//! Full implementation will be done in task 6.

pub mod db;
pub mod error;

pub use db::Database;
pub use error::StorageError;
