//! CLI argument definitions using clap

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "f1r3fly-rgb-wallet",
    version,
    about = "F1r3fly-RGB Wallet - Bitcoin wallet with RGB smart contracts support",
    long_about = None
)]
pub struct Cli {
    /// Wallet name to use (overrides config)
    #[arg(short, long, global = true)]
    pub wallet: Option<String>,

    /// Network to use: regtest, signet, testnet, mainnet (overrides config)
    #[arg(short, long, global = true)]
    pub network: Option<String>,

    /// F1r3node host (overrides config)
    #[arg(long, global = true)]
    pub f1r3node_host: Option<String>,

    /// F1r3node gRPC port (overrides config)
    #[arg(long, global = true)]
    pub f1r3node_grpc_port: Option<u16>,

    /// F1r3node HTTP port (overrides config)
    #[arg(long, global = true)]
    pub f1r3node_http_port: Option<u16>,

    /// Esplora server URL (overrides config)
    #[arg(long, global = true)]
    pub esplora_url: Option<String>,

    /// Custom data directory for wallets
    #[arg(long, global = true)]
    pub data_dir: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize or manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Wallet management commands
    Wallet {
        #[command(subcommand)]
        action: WalletAction,
    },

    /// Sync wallet with blockchain
    Sync {
        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Get Bitcoin balance
    GetBalance {
        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Get wallet addresses
    GetAddresses {
        /// Number of addresses to show (default: 5)
        #[arg(short, long, default_value = "5")]
        count: usize,

        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Create a UTXO via self-send
    CreateUtxo {
        /// Amount in BTC
        #[arg(short, long)]
        amount: f64,

        /// Fee rate in sat/vB (optional)
        #[arg(long)]
        fee_rate: Option<f32>,

        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Send Bitcoin to an address
    SendBitcoin {
        /// Destination address
        #[arg(short, long)]
        to: String,

        /// Amount in satoshis
        #[arg(short, long)]
        amount: u64,

        /// Fee rate in sat/vB (optional)
        #[arg(long)]
        fee_rate: Option<f32>,

        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Issue a new RGB asset
    IssueAsset {
        /// Asset ticker symbol (e.g., "USD")
        #[arg(short, long)]
        ticker: String,

        /// Asset full name (e.g., "US Dollar")
        #[arg(short, long)]
        name: String,

        /// Total supply (raw integer, e.g., 100000000)
        #[arg(short, long)]
        supply: u64,

        /// Decimal precision (e.g., 8 for Bitcoin)
        #[arg(long)]
        precision: u8,

        /// Genesis UTXO in format "txid:vout"
        #[arg(short, long)]
        genesis_utxo: String,

        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// List all RGB assets in the wallet
    ListAssets {
        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Get RGB balance for all assets or a specific asset
    RgbBalance {
        /// Optional contract ID (if omitted, shows all assets)
        #[arg(short, long)]
        contract_id: Option<String>,

        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Get detailed information about a specific RGB asset
    GetContractInfo {
        /// Contract ID
        #[arg(short, long)]
        contract_id: String,

        /// Password to decrypt the wallet
        #[arg(short, long)]
        password: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Initialize configuration file with defaults
    Init {
        /// Network to initialize for (defaults to regtest)
        #[arg(short, long)]
        network: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum WalletAction {
    /// Create a new wallet with a generated mnemonic
    Create {
        /// Name of the wallet
        name: String,

        /// Password to encrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// Import an existing wallet from a mnemonic phrase
    Import {
        /// Name of the wallet
        name: String,

        /// 12-word BIP39 mnemonic phrase
        #[arg(short, long)]
        mnemonic: String,

        /// Password to encrypt the wallet
        #[arg(short, long)]
        password: String,
    },

    /// List all wallets
    List,
}

