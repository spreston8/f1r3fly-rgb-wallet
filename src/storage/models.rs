//! Storage data models
//!
//! Defines wallet-related data structures for persistence and user output.

use bitcoin::bip32::Xpriv;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::NetworkType;

/// Complete wallet key material (kept in memory during operations)
///
/// Contains all cryptographic keys needed for wallet operations:
/// - BIP39 mnemonic (source of all keys)
/// - Bitcoin keys (BIP32 extended private key)
/// - F1r3fly keys (custom derivation for F1r3node operations)
///
/// # Security
///
/// This struct should only exist in memory. Sensitive fields (mnemonic, private keys)
/// must be encrypted before saving to disk.
#[derive(Debug, Clone)]
pub struct WalletKeys {
    /// BIP39 mnemonic phrase (12 words)
    pub mnemonic: bip39::Mnemonic,

    /// Bitcoin extended private key at account level (m/84'/coin_type'/0')
    pub bitcoin_xprv: Xpriv,

    /// Bitcoin descriptor for BDK wallet
    pub bitcoin_descriptor: String,

    /// F1r3fly private key (derived at m/1337'/0'/0'/0/0)
    pub f1r3fly_private_key: secp256k1::SecretKey,

    /// F1r3fly public key (hex-encoded, for F1r3node authentication)
    pub f1r3fly_public_key: String,
}

impl WalletKeys {
    /// Derive all keys from a mnemonic
    ///
    /// # Arguments
    ///
    /// * `mnemonic` - BIP39 mnemonic phrase
    /// * `network` - Target Bitcoin network
    ///
    /// # Example
    ///
    /// ```ignore
    /// use f1r3fly_rgb_wallet::storage::models::WalletKeys;
    /// use f1r3fly_rgb_wallet::config::NetworkType;
    ///
    /// let mnemonic = generate_mnemonic()?;
    /// let keys = WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest)?;
    /// ```
    pub fn from_mnemonic(
        mnemonic: &bip39::Mnemonic,
        network: NetworkType,
    ) -> Result<Self, crate::storage::keys::KeyError> {
        // Derive Bitcoin keys
        let bitcoin_xprv = crate::storage::keys::derive_bitcoin_keys(mnemonic, network)?;

        // Create taproot descriptor for BDK (BIP86)
        // tr(...) format is required for Tapret commitments in RGB protocol
        let bitcoin_descriptor = format!("tr({}/0/*)", bitcoin_xprv);

        // Derive F1r3fly keys
        let (f1r3fly_private_key, f1r3fly_public_key) =
            crate::storage::keys::derive_f1r3fly_key(mnemonic)?;

        Ok(Self {
            mnemonic: mnemonic.clone(),
            bitcoin_xprv,
            bitcoin_descriptor,
            f1r3fly_private_key,
            f1r3fly_public_key,
        })
    }

    /// Get the first Bitcoin address (for display purposes)
    ///
    /// Derives the first receive address (m/84'/coin_type'/0'/0/0)
    pub fn first_address(
        &self,
        network: NetworkType,
    ) -> Result<String, crate::storage::keys::KeyError> {
        use bitcoin::bip32::DerivationPath;
        use std::str::FromStr;

        let secp = bitcoin::secp256k1::Secp256k1::new();
        let path = DerivationPath::from_str("m/0/0")
            .map_err(|e| crate::storage::keys::KeyError::Bip32(e.to_string()))?;

        let child_key = self
            .bitcoin_xprv
            .derive_priv(&secp, &path)
            .map_err(|e| crate::storage::keys::KeyError::Bip32(e.to_string()))?;

        let secp_pubkey = child_key.private_key.public_key(&secp);

        // Convert to bitcoin::PublicKey for taproot (BIP86 key-spend path)
        let bitcoin_pubkey = bitcoin::PublicKey::new(secp_pubkey);

        // Map NetworkType to bitcoin::Network
        let btc_network = match network {
            NetworkType::Mainnet => bitcoin::Network::Bitcoin,
            NetworkType::Testnet => bitcoin::Network::Testnet,
            NetworkType::Signet => bitcoin::Network::Signet,
            NetworkType::Regtest => bitcoin::Network::Regtest,
        };

        // Create taproot address using untweaked public key (BIP86 key-spend only)
        // This generates P2TR addresses (bc1p..., tb1p..., bcrt1p...)
        // Convert to x-only public key (taproot uses only the x-coordinate)
        let address = bitcoin::Address::p2tr(&secp, bitcoin_pubkey.inner.into(), None, btc_network);

        Ok(address.to_string())
    }
}

/// Wallet metadata (non-sensitive information)
///
/// Stored unencrypted for fast wallet listing and network verification.
/// Saved to: `~/.f1r3fly-rgb-wallet/wallets/<name>/wallet.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletMetadata {
    /// Wallet name (unique identifier)
    pub name: String,

    /// Network this wallet operates on
    pub network: NetworkType,

    /// When the wallet was created
    pub created_at: DateTime<Utc>,

    /// Last blockchain sync timestamp
    pub last_sync: Option<DateTime<Utc>>,
}

impl WalletMetadata {
    /// Create new metadata for a wallet
    pub fn new(name: String, network: NetworkType) -> Self {
        Self {
            name,
            network,
            created_at: Utc::now(),
            last_sync: None,
        }
    }

    /// Update last sync timestamp
    pub fn update_sync_time(&mut self) {
        self.last_sync = Some(Utc::now());
    }
}

/// User-facing wallet information (returned on creation/import)
///
/// Contains information to display to the user, including sensitive data
/// like the mnemonic (shown once on creation). This is a transient struct
/// and is not persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletInfo {
    /// Wallet name
    pub name: String,

    /// BIP39 mnemonic phrase (12 words)
    /// ⚠️ Sensitive: Only show this once on wallet creation
    pub mnemonic: String,

    /// Network this wallet operates on
    pub network: NetworkType,

    /// First receive address
    pub first_address: String,

    /// Bitcoin descriptor
    pub descriptor: String,

    /// F1r3fly public key (hex)
    pub f1r3fly_public_key: String,
}

impl WalletInfo {
    /// Create WalletInfo from WalletKeys (for user output)
    pub fn from_keys(
        name: String,
        keys: &WalletKeys,
        network: NetworkType,
    ) -> Result<Self, crate::storage::keys::KeyError> {
        Ok(Self {
            name,
            mnemonic: keys.mnemonic.to_string(),
            network,
            first_address: keys.first_address(network)?,
            descriptor: keys.bitcoin_descriptor.clone(),
            f1r3fly_public_key: keys.f1r3fly_public_key.clone(),
        })
    }
}

/// Encrypted wallet keys storage format
///
/// This is what gets saved to disk in `keys.json`.
/// All sensitive fields are encrypted with user's password.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedWalletKeys {
    /// Encrypted BIP39 mnemonic (hex-encoded)
    pub encrypted_mnemonic: String,

    /// Bitcoin descriptor (not encrypted - derived from xprv anyway)
    pub bitcoin_descriptor: String,

    /// F1r3fly public key (not encrypted - it's public)
    pub f1r3fly_public_key: String,

    /// Encrypted F1r3fly private key (hex-encoded)
    pub encrypted_f1r3fly_private_key: String,
}

impl EncryptedWalletKeys {
    /// Encrypt wallet keys with password
    pub fn from_keys(
        keys: &WalletKeys,
        password: &str,
    ) -> Result<Self, crate::storage::keys::KeyError> {
        Ok(Self {
            encrypted_mnemonic: crate::storage::keys::encrypt_mnemonic(&keys.mnemonic, password)?,
            bitcoin_descriptor: keys.bitcoin_descriptor.clone(),
            f1r3fly_public_key: keys.f1r3fly_public_key.clone(),
            encrypted_f1r3fly_private_key: crate::storage::keys::encrypt_secret_key(
                &keys.f1r3fly_private_key,
                password,
            )?,
        })
    }

    /// Decrypt to WalletKeys
    pub fn to_keys(
        &self,
        password: &str,
        network: NetworkType,
    ) -> Result<WalletKeys, crate::storage::keys::KeyError> {
        // Decrypt mnemonic
        let mnemonic = crate::storage::keys::decrypt_mnemonic(&self.encrypted_mnemonic, password)?;

        // Decrypt F1r3fly private key
        let f1r3fly_private_key = crate::storage::keys::decrypt_secret_key(
            &self.encrypted_f1r3fly_private_key,
            password,
        )?;

        // Derive Bitcoin keys from mnemonic
        let bitcoin_xprv = crate::storage::keys::derive_bitcoin_keys(&mnemonic, network)?;

        Ok(WalletKeys {
            mnemonic,
            bitcoin_xprv,
            bitcoin_descriptor: self.bitcoin_descriptor.clone(),
            f1r3fly_private_key,
            f1r3fly_public_key: self.f1r3fly_public_key.clone(),
        })
    }
}
