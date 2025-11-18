//! Shared types for f1r3fly-rgb-wallet
//!
//! Common data structures used across the wallet implementation.

use serde::{Deserialize, Serialize};

/// Information about a single UTXO with RGB metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoInfo {
    /// UTXO outpoint in format "txid:vout"
    pub outpoint: String,

    /// Transaction ID
    pub txid: String,

    /// Output index
    pub vout: u32,

    /// Amount in satoshis
    pub amount_sats: u64,

    /// Amount in BTC (for display)
    pub amount_btc: f64,

    /// Number of confirmations (0 if unconfirmed)
    pub confirmations: u32,

    /// UTXO status (Available, RGB-Occupied, or Unconfirmed)
    pub status: UtxoStatus,

    /// RGB assets bound to this UTXO (empty if not occupied)
    pub rgb_assets: Vec<RgbSealInfo>,
}

/// Status of a UTXO
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtxoStatus {
    /// UTXO is available for spending (no RGB assets)
    Available,

    /// UTXO holds RGB assets (should not be spent for regular Bitcoin transactions)
    RgbOccupied,

    /// UTXO is unconfirmed (not yet in a block)
    Unconfirmed,
}

/// RGB asset information for a seal
///
/// Note: Based on traditional RGB wallet analysis, seal type (genesis/transfer/change)
/// is not tracked explicitly. The system only needs to know if a UTXO is occupied
/// and which assets it holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbSealInfo {
    /// RGB contract ID
    pub contract_id: String,

    /// Asset ticker (e.g. "TEST", "USDT")
    pub ticker: String,

    /// Token amount held by this seal (None if amount unknown)
    pub amount: Option<u64>,
}

/// Filter options for UTXO listing
#[derive(Debug, Clone, Default)]
pub struct UtxoFilter {
    /// Only show available (non-RGB) UTXOs
    pub available_only: bool,

    /// Only show RGB-occupied UTXOs
    pub rgb_only: bool,

    /// Only show confirmed UTXOs (default true for safety)
    pub confirmed_only: bool,

    /// Minimum amount in satoshis
    pub min_amount_sats: Option<u64>,
}

/// Output format for UTXO listing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable table format (default)
    Table,

    /// JSON format for machine parsing
    Json,

    /// Compact format for shell scripts: "txid:vout amount status"
    Compact,
}

impl std::fmt::Display for UtxoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UtxoStatus::Available => write!(f, "Available"),
            UtxoStatus::RgbOccupied => write!(f, "RGB-Occupied"),
            UtxoStatus::Unconfirmed => write!(f, "Unconfirmed"),
        }
    }
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" => Ok(OutputFormat::Table),
            "json" => Ok(OutputFormat::Json),
            "compact" => Ok(OutputFormat::Compact),
            _ => Err(format!(
                "Invalid output format '{}'. Valid options: table, json, compact",
                s
            )),
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Table => write!(f, "table"),
            OutputFormat::Json => write!(f, "json"),
            OutputFormat::Compact => write!(f, "compact"),
        }
    }
}
