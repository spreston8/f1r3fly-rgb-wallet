//! CLI command implementations

pub mod bitcoin;
pub mod config;
pub mod invoice;
pub mod rgb;
pub mod transfer;
pub mod wallet;

pub use invoice::{generate_invoice_cmd, parse_invoice_cmd};
