//! Bitcoin wallet layer using BDK
//!
//! Handles Bitcoin UTXO management, blockchain sync, and transaction operations

pub mod balance;
pub mod network;
pub mod sync;
pub mod utxo;
pub mod wallet;

pub use balance::{
    get_addresses, get_balance, get_unused_addresses, is_rgb_occupied, list_utxos,
    mark_rgb_occupied, unmark_rgb_occupied, AddressInfo, Balance, BalanceError, UtxoInfo,
};
pub use network::{default_esplora_url, EsploraClient, NetworkError};
pub use sync::{sync_wallet, sync_wallet_with_progress, SyncError, SyncResult};
pub use utxo::{
    create_utxo, estimate_fee, get_recommended_fee_rates, unlock_utxo, FeeRateConfig, UtxoError,
    UtxoOperationResult,
};
pub use wallet::{BitcoinWallet, BitcoinWalletError};

