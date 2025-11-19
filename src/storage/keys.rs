//! Key derivation and encryption utilities
//!
//! Provides BIP39 mnemonic generation, BIP32 Bitcoin key derivation,
//! custom F1r3fly key derivation, and AES-GCM encryption for secure storage.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::Network as BitcoinNetwork;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use secp256k1::{PublicKey, SecretKey};
use sha2::Sha256;
use std::str::FromStr;

use crate::config::NetworkType;

/// Key derivation and encryption errors
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("BIP39 error: {0}")]
    Bip39(String),

    #[error("BIP32 derivation error: {0}")]
    Bip32(String),

    #[error("Secp256k1 error: {0}")]
    Secp256k1(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: {0}")]
    Decryption(String),
}

/// Generate a new BIP39 mnemonic (12 words)
///
/// Creates a 128-bit entropy mnemonic phrase for wallet key derivation.
///
/// # Returns
///
/// A 12-word BIP39 mnemonic phrase
///
/// # Example
///
/// ```ignore
/// let mnemonic = generate_mnemonic()?;
/// println!("Mnemonic: {}", mnemonic);
/// ```
pub fn generate_mnemonic() -> Result<bip39::Mnemonic, KeyError> {
    // Generate 128 bits (16 bytes) of entropy for 12-word mnemonic
    let mut entropy = [0u8; 16];
    OsRng.fill_bytes(&mut entropy);

    bip39::Mnemonic::from_entropy(&entropy).map_err(|e| KeyError::Bip39(e.to_string()))
}

/// Derive Bitcoin keys from mnemonic using BIP32 at path m/86'/x'/0'
///
/// Uses BIP86 derivation path for taproot (P2TR):
/// - Mainnet: m/86'/0'/0'
/// - Testnet: m/86'/1'/0'
/// - Signet: m/86'/1'/0' (uses testnet coin type)
/// - Regtest: m/86'/1'/0' (uses testnet coin type)
///
/// BIP86 is specifically designed for single-key P2TR (Pay-to-Taproot) outputs,
/// which are required for RGB Tapret commitments.
///
/// # Arguments
///
/// * `mnemonic` - BIP39 mnemonic phrase
/// * `network` - Target network type
///
/// # Returns
///
/// Extended private key (xprv) at the account level
///
/// # Example
///
/// ```ignore
/// let mnemonic = generate_mnemonic()?;
/// let xprv = derive_bitcoin_keys(&mnemonic, NetworkType::Regtest)?;
/// ```
pub fn derive_bitcoin_keys(
    mnemonic: &bip39::Mnemonic,
    network: NetworkType,
) -> Result<Xpriv, KeyError> {
    // Convert to seed
    let seed = mnemonic.to_seed("");

    // Map NetworkType to bitcoin::Network
    let btc_network = match network {
        NetworkType::Mainnet => BitcoinNetwork::Bitcoin,
        NetworkType::Testnet => BitcoinNetwork::Testnet,
        NetworkType::Signet => BitcoinNetwork::Signet,
        NetworkType::Regtest => BitcoinNetwork::Regtest,
    };

    // Create master key from seed
    let secp = Secp256k1::new();
    let master_key = Xpriv::new_master(btc_network, &seed)
        .map_err(|e| KeyError::Bip32(format!("Failed to create master key: {}", e)))?;

    // BIP86 path: m/86'/coin_type'/0'
    // coin_type: 0 for mainnet, 1 for testnet/signet/regtest
    let coin_type = match network {
        NetworkType::Mainnet => "0",
        NetworkType::Testnet | NetworkType::Signet | NetworkType::Regtest => "1",
    };

    let path_str = format!("m/86'/{}'/{}'", coin_type, 0);
    let derivation_path = DerivationPath::from_str(&path_str)
        .map_err(|e| KeyError::Bip32(format!("Invalid derivation path: {}", e)))?;

    // Derive to account level
    let account_key = master_key
        .derive_priv(&secp, &derivation_path)
        .map_err(|e| KeyError::Bip32(format!("Derivation failed: {}", e)))?;

    Ok(account_key)
}

/// Derive F1r3fly key from mnemonic at custom path m/1337'/0'/0'/0/0
///
/// Derives a single secp256k1 keypair for F1r3node operations.
/// Uses a custom derivation path to separate from Bitcoin keys.
///
/// # Arguments
///
/// * `mnemonic` - BIP39 mnemonic phrase
///
/// # Returns
///
/// Tuple of (private_key, public_key_hex) for F1r3node authentication
///
/// # Example
///
/// ```ignore
/// let mnemonic = generate_mnemonic()?;
/// let (privkey, pubkey_hex) = derive_f1r3fly_key(&mnemonic)?;
/// ```
pub fn derive_f1r3fly_key(mnemonic: &bip39::Mnemonic) -> Result<(SecretKey, String), KeyError> {
    // Convert to seed
    let seed = mnemonic.to_seed("");

    // Create master key (using Bitcoin's BIP32 with testnet for path derivation)
    let secp = Secp256k1::new();
    let master_key = Xpriv::new_master(BitcoinNetwork::Testnet, &seed)
        .map_err(|e| KeyError::Bip32(format!("Failed to create master key: {}", e)))?;

    // Custom F1r3fly path: m/1337'/0'/0'/0/0
    let path_str = "m/1337'/0'/0'/0/0";
    let derivation_path = DerivationPath::from_str(path_str)
        .map_err(|e| KeyError::Bip32(format!("Invalid derivation path: {}", e)))?;

    // Derive the key
    let derived_key = master_key
        .derive_priv(&secp, &derivation_path)
        .map_err(|e| KeyError::Bip32(format!("F1r3fly key derivation failed: {}", e)))?;

    // Extract secp256k1 keys
    let private_key = derived_key.private_key;
    let secret_key = SecretKey::from_slice(private_key.as_ref())
        .map_err(|e| KeyError::Secp256k1(format!("Invalid secret key: {}", e)))?;

    // Derive public key
    let secp_ctx = secp256k1::Secp256k1::new();
    let public_key = PublicKey::from_secret_key(&secp_ctx, &secret_key);
    let public_key_hex = hex::encode(public_key.serialize());

    Ok((secret_key, public_key_hex))
}

/// Encrypt data using AES-256-GCM with password-derived key
///
/// Uses production-grade password-based encryption:
/// - PBKDF2-HMAC-SHA256 with 600,000 iterations
/// - Random 128-bit salt
/// - Random 96-bit nonce for each encryption
/// - Returns: salt (16 bytes) || nonce (12 bytes) || ciphertext || tag (16 bytes)
///
/// # Arguments
///
/// * `data` - Plaintext bytes to encrypt
/// * `password` - Password for encryption
///
/// # Returns
///
/// Encrypted data as hex string (salt + nonce + ciphertext + tag)
///
/// # Example
///
/// ```ignore
/// let encrypted = encrypt_data(b"secret", "my_password")?;
/// let decrypted = decrypt_data(&encrypted, "my_password")?;
/// ```
pub fn encrypt_data(data: &[u8], password: &str) -> Result<String, KeyError> {
    // Generate random salt (128 bits / 16 bytes)
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);

    // Derive 256-bit key from password using PBKDF2-HMAC-SHA256
    // 600,000 iterations (OWASP recommendation as of 2023)
    let mut key_bytes = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, 600_000, &mut key_bytes);
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);

    // Create cipher
    let cipher = Aes256Gcm::new(key);

    // Generate random nonce (96 bits / 12 bytes)
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt
    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| KeyError::Encryption(e.to_string()))?;

    // Combine: salt || nonce || ciphertext
    let mut result = salt.to_vec();
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    // Return as hex string
    Ok(hex::encode(result))
}

/// Decrypt data encrypted with encrypt_data()
///
/// # Arguments
///
/// * `encrypted_hex` - Hex-encoded encrypted data (salt + nonce + ciphertext + tag)
/// * `password` - Password used for encryption
///
/// # Returns
///
/// Decrypted plaintext bytes
///
/// # Example
///
/// ```ignore
/// let encrypted = encrypt_data(b"secret", "password")?;
/// let decrypted = decrypt_data(&encrypted, "password")?;
/// assert_eq!(decrypted, b"secret");
/// ```
pub fn decrypt_data(encrypted_hex: &str, password: &str) -> Result<Vec<u8>, KeyError> {
    // Decode hex
    let encrypted_bytes =
        hex::decode(encrypted_hex).map_err(|e| KeyError::Decryption(e.to_string()))?;

    // Minimum size: salt (16) + nonce (12) + tag (16) = 44 bytes
    if encrypted_bytes.len() < 44 {
        return Err(KeyError::Decryption(
            "Data too short (minimum 44 bytes required)".to_string(),
        ));
    }

    // Extract salt (first 16 bytes)
    let (salt, rest) = encrypted_bytes.split_at(16);

    // Extract nonce (next 12 bytes)
    let (nonce_bytes, ciphertext) = rest.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Derive key from password using same PBKDF2 parameters
    let mut key_bytes = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, 600_000, &mut key_bytes);
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);

    // Create cipher
    let cipher = Aes256Gcm::new(key);

    // Decrypt
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| KeyError::Decryption(format!("Decryption failed (wrong password?): {}", e)))?;

    Ok(plaintext)
}

/// Encrypt mnemonic phrase for secure storage
///
/// Convenience wrapper around encrypt_data for mnemonics.
///
/// # Arguments
///
/// * `mnemonic` - BIP39 mnemonic to encrypt
/// * `password` - Password for encryption
///
/// # Returns
///
/// Hex-encoded encrypted mnemonic
pub fn encrypt_mnemonic(mnemonic: &bip39::Mnemonic, password: &str) -> Result<String, KeyError> {
    let mnemonic_str = mnemonic.to_string();
    encrypt_data(mnemonic_str.as_bytes(), password)
}

/// Decrypt and parse mnemonic phrase
///
/// Convenience wrapper around decrypt_data for mnemonics.
///
/// # Arguments
///
/// * `encrypted_hex` - Hex-encoded encrypted mnemonic
/// * `password` - Password used for encryption
///
/// # Returns
///
/// Parsed BIP39 mnemonic
pub fn decrypt_mnemonic(encrypted_hex: &str, password: &str) -> Result<bip39::Mnemonic, KeyError> {
    let decrypted_bytes = decrypt_data(encrypted_hex, password)?;
    let mnemonic_str =
        String::from_utf8(decrypted_bytes).map_err(|e| KeyError::Decryption(e.to_string()))?;

    bip39::Mnemonic::parse(&mnemonic_str).map_err(|e| KeyError::Bip39(e.to_string()))
}

/// Encrypt secp256k1 secret key for secure storage
///
/// # Arguments
///
/// * `secret_key` - secp256k1 secret key to encrypt
/// * `password` - Password for encryption
///
/// # Returns
///
/// Hex-encoded encrypted secret key
pub fn encrypt_secret_key(secret_key: &SecretKey, password: &str) -> Result<String, KeyError> {
    let secret_bytes = secret_key.secret_bytes();
    encrypt_data(&secret_bytes, password)
}

/// Decrypt secp256k1 secret key
///
/// # Arguments
///
/// * `encrypted_hex` - Hex-encoded encrypted secret key
/// * `password` - Password used for encryption
///
/// # Returns
///
/// Decrypted secp256k1 secret key
pub fn decrypt_secret_key(encrypted_hex: &str, password: &str) -> Result<SecretKey, KeyError> {
    let decrypted_bytes = decrypt_data(encrypted_hex, password)?;

    SecretKey::from_slice(&decrypted_bytes)
        .map_err(|e| KeyError::Secp256k1(format!("Invalid secret key: {}", e)))
}
