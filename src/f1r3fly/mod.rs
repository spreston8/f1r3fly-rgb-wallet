//! F1r3fly-RGB integration layer
//!
//! This module provides the interface between the wallet and f1r3fly-rgb,
//! managing F1r3node connections, contract state, and RGB operations.

pub mod asset;
pub mod balance;
pub mod consignment;
pub mod contracts;
pub mod executor;
pub mod invoice;
pub mod transfer;

// Re-exports
pub use asset::{
    get_asset_info, issue_asset, list_assets, AssetError, AssetInfo, AssetListItem,
    IssueAssetRequest,
};
pub use balance::{
    get_asset_balance, get_occupied_utxos, get_rgb_balance, get_rgb_seal_info, AssetBalance,
    BalanceError, RgbOccupiedUtxo, UtxoBalance,
};
pub use contracts::{
    ContractsManagerError, F1r3flyContractsManager, F1r3flyState, GenesisExecutionData,
    GenesisUtxoInfo,
};
pub use executor::F1r3flyExecutorManager;
pub use invoice::{
    extract_seal_from_invoice, generate_invoice, get_address_from_invoice, parse_invoice,
    InvoiceError,
};

pub use consignment::{
    accept_consignment, export_genesis, AcceptConsignmentResponse, ConsignmentError,
    ExportGenesisResponse,
};

pub use transfer::{send_transfer, TransferError, TransferResponse};

// Re-export core library types for convenience
pub use f1r3fly_rgb::{GeneratedInvoice, ParsedInvoice, RgbBeneficiary, RgbInvoice};
