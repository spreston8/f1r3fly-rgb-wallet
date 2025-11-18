//! File system operations for wallet persistence
//!
//! Manages wallet directory structure, saving/loading encrypted keys,
//! and wallet metadata.

use std::fs;
use std::path::PathBuf;

use crate::storage::keys::KeyError;
use crate::storage::models::{EncryptedWalletKeys, WalletKeys, WalletMetadata};

/// File system errors
#[derive(Debug, thiserror::Error)]
pub enum FileSystemError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Key error: {0}")]
    Key(#[from] KeyError),

    #[error("Wallet already exists: {0}")]
    WalletExists(String),

    #[error("Wallet not found: {0}")]
    WalletNotFound(String),

    #[error("Wallets directory not found")]
    WalletsDirectoryNotFound,
}

/// Get the default wallets directory path
///
/// Returns: `~/.f1r3fly-rgb-wallet/wallets/`
pub fn default_wallets_dir() -> Result<PathBuf, FileSystemError> {
    let config_dir = crate::config::default_config_dir()
        .map_err(|_| FileSystemError::WalletsDirectoryNotFound)?;
    Ok(config_dir.join("wallets"))
}

/// Get the wallets directory (custom or default)
pub fn wallets_dir(custom_dir: Option<&str>) -> Result<PathBuf, FileSystemError> {
    match custom_dir {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => default_wallets_dir(),
    }
}

/// Get the directory path for a specific wallet
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `custom_base` - Optional custom base directory (for testing)
///
/// Returns: `<base>/wallets/<wallet_name>/` or `~/.f1r3fly-rgb-wallet/wallets/<wallet_name>/`
pub fn wallet_dir(wallet_name: &str, custom_base: Option<&str>) -> Result<PathBuf, FileSystemError> {
    Ok(wallets_dir(custom_base)?.join(wallet_name))
}

/// Create wallet directory structure
///
/// Creates:
/// - `<base>/wallets/<wallet_name>/` or `~/.f1r3fly-rgb-wallet/wallets/<wallet_name>/`
/// - Subdirectories as needed
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `custom_base` - Optional custom base directory (for testing)
///
/// # Errors
///
/// Returns error if:
/// - Wallet directory already exists
/// - Cannot create directories
///
/// # Example
///
/// ```ignore
/// create_wallet_directory("my_wallet", None)?;
/// ```
pub fn create_wallet_directory(wallet_name: &str, custom_base: Option<&str>) -> Result<PathBuf, FileSystemError> {
    let wallet_path = wallet_dir(wallet_name, custom_base)?;

    // Check if wallet already exists
    if wallet_path.exists() {
        return Err(FileSystemError::WalletExists(wallet_name.to_string()));
    }

    // Create wallet directory
    fs::create_dir_all(&wallet_path)?;

    Ok(wallet_path)
}

/// Save wallet to disk
///
/// Saves:
/// 1. Encrypted keys to `keys.json`
/// 2. Metadata to `wallet.json`
/// 3. Bitcoin descriptor to `descriptor.txt`
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `keys` - Wallet keys to save
/// * `metadata` - Wallet metadata
/// * `password` - Password for encryption
/// * `custom_base` - Optional custom base directory (for testing)
///
/// # Example
///
/// ```ignore
/// let keys = WalletKeys::from_mnemonic(&mnemonic, network)?;
/// let metadata = WalletMetadata::new("my_wallet".to_string(), network);
/// save_wallet("my_wallet", &keys, &metadata, "password", None)?;
/// ```
pub fn save_wallet(
    wallet_name: &str,
    keys: &WalletKeys,
    metadata: &WalletMetadata,
    password: &str,
    custom_base: Option<&str>,
) -> Result<(), FileSystemError> {
    let wallet_path = wallet_dir(wallet_name, custom_base)?;

    // Ensure directory exists
    if !wallet_path.exists() {
        fs::create_dir_all(&wallet_path)?;
    }

    // 1. Encrypt and save keys
    let encrypted_keys = EncryptedWalletKeys::from_keys(keys, password)?;
    let keys_json = serde_json::to_string_pretty(&encrypted_keys)?;
    fs::write(wallet_path.join("keys.json"), keys_json)?;

    // 2. Save metadata (unencrypted)
    let metadata_json = serde_json::to_string_pretty(metadata)?;
    fs::write(wallet_path.join("wallet.json"), metadata_json)?;

    // 3. Save Bitcoin descriptor (for BDK)
    fs::write(
        wallet_path.join("descriptor.txt"),
        &keys.bitcoin_descriptor,
    )?;

    Ok(())
}

/// Load wallet from disk
///
/// Loads and decrypts wallet keys and metadata.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `password` - Password for decryption
/// * `custom_base` - Optional custom base directory (for testing)
///
/// # Returns
///
/// Tuple of (WalletKeys, WalletMetadata)
///
/// # Errors
///
/// Returns error if:
/// - Wallet not found
/// - Wrong password
/// - Corrupted files
///
/// # Example
///
/// ```ignore
/// let (keys, metadata) = load_wallet("my_wallet", "password", None)?;
/// ```
pub fn load_wallet(
    wallet_name: &str,
    password: &str,
    custom_base: Option<&str>,
) -> Result<(WalletKeys, WalletMetadata), FileSystemError> {
    let wallet_path = wallet_dir(wallet_name, custom_base)?;

    // Check if wallet exists
    if !wallet_path.exists() {
        return Err(FileSystemError::WalletNotFound(wallet_name.to_string()));
    }

    // 1. Load metadata
    let metadata_path = wallet_path.join("wallet.json");
    let metadata_json = fs::read_to_string(metadata_path)?;
    let metadata: WalletMetadata = serde_json::from_str(&metadata_json)?;

    // 2. Load and decrypt keys
    let keys_path = wallet_path.join("keys.json");
    let keys_json = fs::read_to_string(keys_path)?;
    let encrypted_keys: EncryptedWalletKeys = serde_json::from_str(&keys_json)?;
    let keys = encrypted_keys.to_keys(password, metadata.network)?;

    Ok((keys, metadata))
}

/// List all wallets
///
/// Returns metadata for all wallets found in the wallets directory.
///
/// # Returns
///
/// Vector of WalletMetadata (one per wallet)
///
/// # Example
///
/// ```ignore
/// let wallets = list_wallets(None)?;
/// for wallet in wallets {
///     println!("{} ({})", wallet.name, wallet.network);
/// }
/// ```
pub fn list_wallets(custom_base: Option<&str>) -> Result<Vec<WalletMetadata>, FileSystemError> {
    let wallets_path = wallets_dir(custom_base)?;

    // Create wallets directory if it doesn't exist
    if !wallets_path.exists() {
        fs::create_dir_all(&wallets_path)?;
        return Ok(Vec::new());
    }

    let mut wallets = Vec::new();

    // Iterate through all directories in wallets/
    for entry in fs::read_dir(&wallets_path)? {
        let entry = entry?;
        let path = entry.path();

        // Skip if not a directory
        if !path.is_dir() {
            continue;
        }

        // Try to load wallet.json
        let metadata_path = path.join("wallet.json");
        if metadata_path.exists() {
            match fs::read_to_string(&metadata_path) {
                Ok(json) => match serde_json::from_str::<WalletMetadata>(&json) {
                    Ok(metadata) => wallets.push(metadata),
                    Err(e) => {
                        // Log error but continue
                        eprintln!("Warning: Failed to parse wallet metadata at {:?}: {}", path, e);
                    }
                },
                Err(e) => {
                    eprintln!("Warning: Failed to read wallet metadata at {:?}: {}", path, e);
                }
            }
        }
    }

    // Sort by creation date (newest first)
    wallets.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(wallets)
}

/// Delete wallet from disk
///
/// Removes entire wallet directory and all its contents.
///
/// ⚠️ **WARNING**: This is irreversible! All keys, metadata, and state
/// will be permanently deleted unless backed up elsewhere.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet to delete
///
/// # Errors
///
/// Returns error if:
/// - Wallet not found
/// - Cannot delete files/directory
///
/// # Example
///
/// ```ignore
/// delete_wallet("my_wallet")?;
/// ```
pub fn delete_wallet(wallet_name: &str) -> Result<(), FileSystemError> {
    let wallet_path = wallet_dir(wallet_name, None)?;

    // Check if wallet exists
    if !wallet_path.exists() {
        return Err(FileSystemError::WalletNotFound(wallet_name.to_string()));
    }

    // Remove entire wallet directory
    fs::remove_dir_all(&wallet_path)?;

    Ok(())
}

/// Check if a wallet exists
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
///
/// # Returns
///
/// `true` if wallet directory exists with valid wallet.json, `false` otherwise
pub fn wallet_exists(wallet_name: &str, custom_base: Option<&str>) -> bool {
    if let Ok(wallet_path) = wallet_dir(wallet_name, custom_base) {
        wallet_path.exists() && wallet_path.join("wallet.json").exists()
    } else {
        false
    }
}

/// Load wallet metadata without decrypting keys
///
/// Useful for listing wallets or checking network without password.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
///
/// # Example
///
/// ```ignore
/// let metadata = load_wallet_metadata("my_wallet")?;
/// println!("Network: {:?}", metadata.network);
/// ```
pub fn load_wallet_metadata(wallet_name: &str) -> Result<WalletMetadata, FileSystemError> {
    let wallet_path = wallet_dir(wallet_name, None)?;

    if !wallet_path.exists() {
        return Err(FileSystemError::WalletNotFound(wallet_name.to_string()));
    }

    let metadata_path = wallet_path.join("wallet.json");
    let metadata_json = fs::read_to_string(metadata_path)?;
    let metadata: WalletMetadata = serde_json::from_str(&metadata_json)?;

    Ok(metadata)
}

/// Update wallet metadata on disk
///
/// Used to update last_sync timestamp or other metadata fields.
///
/// # Arguments
///
/// * `wallet_name` - Name of the wallet
/// * `metadata` - Updated metadata
///
/// # Example
///
/// ```ignore
/// let mut metadata = load_wallet_metadata("my_wallet")?;
/// metadata.update_sync_time();
/// update_wallet_metadata("my_wallet", &metadata)?;
/// ```
pub fn update_wallet_metadata(
    wallet_name: &str,
    metadata: &WalletMetadata,
) -> Result<(), FileSystemError> {
    let wallet_path = wallet_dir(wallet_name, None)?;

    if !wallet_path.exists() {
        return Err(FileSystemError::WalletNotFound(wallet_name.to_string()));
    }

    let metadata_json = serde_json::to_string_pretty(metadata)?;
    fs::write(wallet_path.join("wallet.json"), metadata_json)?;

    Ok(())
}

