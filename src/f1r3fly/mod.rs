//! F1r3fly-RGB integration layer
//!
//! This module provides the interface between the wallet and f1r3fly-rgb,
//! managing F1r3node connections, contract state, and RGB operations.

pub mod asset;
pub mod balance;
pub mod contracts;
pub mod executor;

// Re-exports
pub use asset::{
    get_asset_info, issue_asset, list_assets, AssetError, AssetInfo, AssetListItem,
    IssueAssetRequest,
};
pub use balance::{
    get_asset_balance, get_occupied_utxos, get_rgb_balance, AssetBalance, BalanceError,
    RgbOccupiedUtxo, UtxoBalance,
};
pub use contracts::{ContractsManagerError, F1r3flyContractsManager, F1r3flyState, GenesisUtxoInfo};
pub use executor::F1r3flyExecutorManager;

