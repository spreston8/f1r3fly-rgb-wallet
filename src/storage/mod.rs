//! Storage layer for wallet data
//!
//! Manages key derivation, encryption, and persistence.

pub mod claim_storage;
pub mod file_system;
pub mod keys;
pub mod models;

// Re-export claim storage types for external use
pub use claim_storage::{ClaimStatus, ClaimStorage, PendingClaim, StorageError};
