//! Core of AI Usage Tracker: source discovery, tolerant log parsers, the local SQLite archive
//! and incremental ingestion. Independent of the UI shell so it can be tested in isolation.

pub mod analytics;
pub mod discovery;
pub mod ingest;
pub mod model;
pub mod pricing;
pub mod sources;
pub mod store;
