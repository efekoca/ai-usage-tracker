//! Core of AI Usage Tracker: source discovery, tolerant log parsers, the local SQLite archive
//! and incremental ingestion. Independent of the UI shell so it can be tested in isolation.

pub mod analytics;
pub mod capture;
pub mod discovery;
pub mod export;
pub mod history;
pub mod ingest;
pub mod insights;
pub mod model;
pub mod plans;
pub mod pricing;
pub mod sources;
pub mod store;
pub mod tips;

pub use rusqlite;
