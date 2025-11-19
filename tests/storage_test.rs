//! Integration tests for storage models and file system operations
//!
//! Tests wallet creation, persistence, loading, listing, and deletion
//! with proper cleanup of temporary test directories.

use bip39::Mnemonic;
use f1r3fly_rgb_wallet::config::NetworkType;
use f1r3fly_rgb_wallet::storage::file_system::FileSystemError;
use f1r3fly_rgb_wallet::storage::keys::{generate_mnemonic, KeyError};
use f1r3fly_rgb_wallet::storage::models::{WalletInfo, WalletKeys, WalletMetadata};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Test environment with automatic cleanup
struct TestWalletsEnv {
    _temp_dir: TempDir,
    wallets_path: PathBuf,
}

impl TestWalletsEnv {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let wallets_path = temp_dir.path().join("wallets");
        fs::create_dir_all(&wallets_path).expect("Failed to create wallets directory");

        Self {
            _temp_dir: temp_dir,
            wallets_path,
        }
    }

    fn wallet_path(&self, name: &str) -> PathBuf {
        self.wallets_path.join(name)
    }

    fn create_wallet_dir(&self, name: &str) -> PathBuf {
        let wallet_path = self.wallet_path(name);
        fs::create_dir_all(&wallet_path).expect("Failed to create wallet directory");
        wallet_path
    }

    fn save_wallet(
        &self,
        name: &str,
        keys: &WalletKeys,
        metadata: &WalletMetadata,
        password: &str,
    ) -> Result<(), FileSystemError> {
        use f1r3fly_rgb_wallet::storage::models::EncryptedWalletKeys;

        let wallet_path = self.create_wallet_dir(name);

        // Encrypt and save keys
        let encrypted_keys = EncryptedWalletKeys::from_keys(keys, password)?;
        let keys_json = serde_json::to_string_pretty(&encrypted_keys)?;
        fs::write(wallet_path.join("keys.json"), keys_json)?;

        // Save metadata
        let metadata_json = serde_json::to_string_pretty(metadata)?;
        fs::write(wallet_path.join("wallet.json"), metadata_json)?;

        // Save descriptor
        fs::write(wallet_path.join("descriptor.txt"), &keys.bitcoin_descriptor)?;

        Ok(())
    }

    fn load_wallet(
        &self,
        name: &str,
        password: &str,
    ) -> Result<(WalletKeys, WalletMetadata), FileSystemError> {
        use f1r3fly_rgb_wallet::storage::models::EncryptedWalletKeys;

        let wallet_path = self.wallet_path(name);

        if !wallet_path.exists() {
            return Err(FileSystemError::WalletNotFound(name.to_string()));
        }

        // Load metadata
        let metadata_json = fs::read_to_string(wallet_path.join("wallet.json"))?;
        let metadata: WalletMetadata = serde_json::from_str(&metadata_json)?;

        // Load and decrypt keys
        let keys_json = fs::read_to_string(wallet_path.join("keys.json"))?;
        let encrypted_keys: EncryptedWalletKeys = serde_json::from_str(&keys_json)?;
        let keys = encrypted_keys.to_keys(password, metadata.network)?;

        Ok((keys, metadata))
    }

    fn list_wallets(&self) -> Result<Vec<WalletMetadata>, FileSystemError> {
        let mut wallets = Vec::new();

        for entry in fs::read_dir(&self.wallets_path)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let metadata_path = path.join("wallet.json");
            if metadata_path.exists() {
                let json = fs::read_to_string(&metadata_path)?;
                let metadata: WalletMetadata = serde_json::from_str(&json)?;
                wallets.push(metadata);
            }
        }

        wallets.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(wallets)
    }

    fn delete_wallet(&self, name: &str) -> Result<(), FileSystemError> {
        let wallet_path = self.wallet_path(name);

        if !wallet_path.exists() {
            return Err(FileSystemError::WalletNotFound(name.to_string()));
        }

        fs::remove_dir_all(&wallet_path)?;
        Ok(())
    }

    fn wallet_exists(&self, name: &str) -> bool {
        let wallet_path = self.wallet_path(name);
        wallet_path.exists() && wallet_path.join("wallet.json").exists()
    }
}

#[test]
fn test_wallet_creation_and_round_trip_encryption() {
    let env = TestWalletsEnv::new();
    let password = "secure_password_123";
    let wallet_name = "test_wallet";

    // 1. Generate mnemonic and derive keys
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");

    // Verify keys are correctly derived
    assert_eq!(keys.mnemonic.to_string(), mnemonic.to_string());
    assert!(keys.bitcoin_descriptor.starts_with("tr("));
    assert_eq!(keys.f1r3fly_public_key.len(), 66); // 33 bytes * 2 hex chars
    assert!(keys.f1r3fly_public_key.starts_with("02") || keys.f1r3fly_public_key.starts_with("03"));

    // Verify first address can be derived
    let first_address = keys
        .first_address(NetworkType::Regtest)
        .expect("Failed to derive first address");
    assert!(
        first_address.starts_with("bcrt1"),
        "Regtest address should start with bcrt1"
    );

    // 2. Create metadata
    let metadata = WalletMetadata::new(wallet_name.to_string(), NetworkType::Regtest);
    assert_eq!(metadata.name, wallet_name);
    assert_eq!(metadata.network, NetworkType::Regtest);
    assert!(metadata.last_sync.is_none());

    // 3. Save wallet
    env.save_wallet(wallet_name, &keys, &metadata, password)
        .expect("Failed to save wallet");

    // Verify files were created
    assert!(env.wallet_path(wallet_name).exists());
    assert!(env.wallet_path(wallet_name).join("keys.json").exists());
    assert!(env.wallet_path(wallet_name).join("wallet.json").exists());
    assert!(env.wallet_path(wallet_name).join("descriptor.txt").exists());

    // 4. Load wallet back with correct password
    let (loaded_keys, loaded_metadata) = env
        .load_wallet(wallet_name, password)
        .expect("Failed to load wallet");

    // Verify loaded keys match original
    assert_eq!(loaded_keys.mnemonic.to_string(), keys.mnemonic.to_string());
    assert_eq!(loaded_keys.bitcoin_descriptor, keys.bitcoin_descriptor);
    assert_eq!(loaded_keys.f1r3fly_public_key, keys.f1r3fly_public_key);
    assert_eq!(
        loaded_keys.f1r3fly_private_key.secret_bytes(),
        keys.f1r3fly_private_key.secret_bytes()
    );

    // Verify loaded metadata matches original
    assert_eq!(loaded_metadata.name, metadata.name);
    assert_eq!(loaded_metadata.network, metadata.network);
    assert_eq!(loaded_metadata.created_at, metadata.created_at);

    // 5. Verify WalletInfo can be created
    let wallet_info =
        WalletInfo::from_keys(wallet_name.to_string(), &loaded_keys, NetworkType::Regtest)
            .expect("Failed to create WalletInfo");
    assert_eq!(wallet_info.name, wallet_name);
    assert_eq!(wallet_info.mnemonic, mnemonic.to_string());
    assert_eq!(wallet_info.network, NetworkType::Regtest);
    assert_eq!(wallet_info.descriptor, keys.bitcoin_descriptor);
    assert_eq!(wallet_info.f1r3fly_public_key, keys.f1r3fly_public_key);
    assert!(wallet_info.first_address.starts_with("bcrt1"));
}

#[test]
fn test_wallet_decryption_fails_with_wrong_password() {
    let env = TestWalletsEnv::new();
    let correct_password = "correct_password";
    let wrong_password = "wrong_password";
    let wallet_name = "secure_wallet";

    // Create and save wallet
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");
    let metadata = WalletMetadata::new(wallet_name.to_string(), NetworkType::Regtest);

    env.save_wallet(wallet_name, &keys, &metadata, correct_password)
        .expect("Failed to save wallet");

    // Attempt to load with wrong password
    let result = env.load_wallet(wallet_name, wrong_password);

    assert!(result.is_err(), "Loading with wrong password should fail");

    match result {
        Err(FileSystemError::Key(KeyError::Decryption(msg))) => {
            assert!(
                msg.contains("wrong password") || msg.contains("Decryption failed"),
                "Error should indicate decryption failure, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Key(Decryption) error, got: {:?}", e),
        Ok(_) => panic!("Should not succeed with wrong password"),
    }

    // Verify correct password still works
    let result = env.load_wallet(wallet_name, correct_password);
    assert!(
        result.is_ok(),
        "Loading with correct password should succeed"
    );

    // Verify attempting to load non-existent wallet fails appropriately
    let result = env.load_wallet("nonexistent_wallet", correct_password);
    assert!(result.is_err(), "Loading non-existent wallet should fail");

    match result {
        Err(FileSystemError::WalletNotFound(name)) => {
            assert_eq!(name, "nonexistent_wallet");
        }
        _ => panic!("Should return WalletNotFound error"),
    }
}

#[test]
fn test_listing_multiple_wallets_sorted_by_creation_date() {
    let env = TestWalletsEnv::new();
    let password = "password";

    // Initially, no wallets should exist
    let wallets = env.list_wallets().expect("Failed to list wallets");
    assert_eq!(wallets.len(), 0, "Should start with no wallets");

    // Create multiple wallets with slight delays to ensure different timestamps
    let wallet_names = vec!["wallet1", "wallet2", "wallet3"];
    let mut created_wallets = Vec::new();

    for (i, name) in wallet_names.iter().enumerate() {
        let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
        let keys = WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest)
            .expect("Failed to derive keys");

        // Use different networks to verify metadata is preserved
        let network = match i {
            0 => NetworkType::Regtest,
            1 => NetworkType::Signet,
            2 => NetworkType::Testnet,
            _ => NetworkType::Regtest,
        };

        let metadata = WalletMetadata::new(name.to_string(), network);
        created_wallets.push((name.to_string(), network, metadata.created_at));

        env.save_wallet(name, &keys, &metadata, password)
            .expect("Failed to save wallet");

        // Small delay to ensure different timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // List wallets
    let wallets = env.list_wallets().expect("Failed to list wallets");
    assert_eq!(wallets.len(), 3, "Should have 3 wallets");

    // Verify all wallets are present
    for (name, network, _) in &created_wallets {
        let found = wallets.iter().find(|w| &w.name == name);
        assert!(found.is_some(), "Wallet {} should be in list", name);
        let found_wallet = found.unwrap();
        assert_eq!(
            found_wallet.network, *network,
            "Network should match for {}",
            name
        );
    }

    // Verify wallets are sorted by creation date (newest first)
    for i in 1..wallets.len() {
        assert!(
            wallets[i - 1].created_at >= wallets[i].created_at,
            "Wallets should be sorted by creation date (newest first)"
        );
    }

    // Verify wallet_exists helper
    for name in &wallet_names {
        assert!(env.wallet_exists(name), "Wallet {} should exist", name);
    }

    assert!(
        !env.wallet_exists("nonexistent"),
        "Nonexistent wallet should not exist"
    );

    // Verify we can load each wallet
    for name in &wallet_names {
        let result = env.load_wallet(name, password);
        assert!(result.is_ok(), "Should be able to load wallet {}", name);
    }
}

#[test]
fn test_deleting_wallet_removes_all_files_and_directory() {
    let env = TestWalletsEnv::new();
    let password = "password";
    let wallet_name = "wallet_to_delete";

    // Create wallet
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let keys =
        WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest).expect("Failed to derive keys");
    let metadata = WalletMetadata::new(wallet_name.to_string(), NetworkType::Regtest);

    env.save_wallet(wallet_name, &keys, &metadata, password)
        .expect("Failed to save wallet");

    // Verify wallet exists
    assert!(
        env.wallet_exists(wallet_name),
        "Wallet should exist before deletion"
    );
    let wallet_path = env.wallet_path(wallet_name);
    assert!(wallet_path.exists(), "Wallet directory should exist");
    assert!(wallet_path.join("keys.json").exists());
    assert!(wallet_path.join("wallet.json").exists());
    assert!(wallet_path.join("descriptor.txt").exists());

    // List wallets - should have 1
    let wallets = env.list_wallets().expect("Failed to list wallets");
    assert_eq!(wallets.len(), 1, "Should have 1 wallet before deletion");

    // Delete wallet
    env.delete_wallet(wallet_name)
        .expect("Failed to delete wallet");

    // Verify wallet no longer exists
    assert!(
        !env.wallet_exists(wallet_name),
        "Wallet should not exist after deletion"
    );
    assert!(!wallet_path.exists(), "Wallet directory should be removed");
    assert!(!wallet_path.join("keys.json").exists());
    assert!(!wallet_path.join("wallet.json").exists());
    assert!(!wallet_path.join("descriptor.txt").exists());

    // List wallets - should have 0
    let wallets = env.list_wallets().expect("Failed to list wallets");
    assert_eq!(wallets.len(), 0, "Should have no wallets after deletion");

    // Attempting to delete again should fail
    let result = env.delete_wallet(wallet_name);
    assert!(result.is_err(), "Deleting non-existent wallet should fail");

    match result {
        Err(FileSystemError::WalletNotFound(name)) => {
            assert_eq!(name, wallet_name);
        }
        _ => panic!("Should return WalletNotFound error"),
    }

    // Attempting to load deleted wallet should fail
    let result = env.load_wallet(wallet_name, password);
    assert!(result.is_err(), "Loading deleted wallet should fail");

    match result {
        Err(FileSystemError::WalletNotFound(name)) => {
            assert_eq!(name, wallet_name);
        }
        _ => panic!("Should return WalletNotFound error"),
    }
}

#[test]
fn test_wallet_keys_derivation_is_deterministic_across_networks() {
    // Use a known test mnemonic
    let mnemonic_str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let mnemonic = Mnemonic::parse(mnemonic_str).expect("Failed to parse mnemonic");

    // Test all networks
    let networks = vec![
        NetworkType::Regtest,
        NetworkType::Signet,
        NetworkType::Testnet,
        NetworkType::Mainnet,
    ];

    for network in networks {
        // Derive keys multiple times - should always produce same result
        let keys1 = WalletKeys::from_mnemonic(&mnemonic, network)
            .expect("Failed to derive keys (attempt 1)");
        let keys2 = WalletKeys::from_mnemonic(&mnemonic, network)
            .expect("Failed to derive keys (attempt 2)");

        // Verify determinism
        assert_eq!(
            keys1.mnemonic.to_string(),
            keys2.mnemonic.to_string(),
            "Mnemonics should match for {:?}",
            network
        );
        assert_eq!(
            keys1.bitcoin_descriptor, keys2.bitcoin_descriptor,
            "Bitcoin descriptors should match for {:?}",
            network
        );
        assert_eq!(
            keys1.f1r3fly_public_key, keys2.f1r3fly_public_key,
            "F1r3fly public keys should match for {:?}",
            network
        );
        assert_eq!(
            keys1.f1r3fly_private_key.secret_bytes(),
            keys2.f1r3fly_private_key.secret_bytes(),
            "F1r3fly private keys should match for {:?}",
            network
        );

        // Verify Bitcoin descriptor has correct format
        assert!(
            keys1.bitcoin_descriptor.starts_with("tr("),
            "Descriptor should be tr (taproot) for {:?}",
            network
        );
        assert!(
            keys1.bitcoin_descriptor.contains("/0/*"),
            "Descriptor should have /0/* suffix for {:?}",
            network
        );

        // Verify first address can be derived and has correct prefix
        let first_address = keys1
            .first_address(network)
            .expect(&format!("Failed to derive first address for {:?}", network));

        let expected_prefix = match network {
            NetworkType::Mainnet => "bc1",
            NetworkType::Testnet => "tb1",
            NetworkType::Signet => "tb1",
            NetworkType::Regtest => "bcrt1",
        };

        assert!(
            first_address.starts_with(expected_prefix),
            "Address should start with {} for {:?}, got: {}",
            expected_prefix,
            network,
            first_address
        );

        // Verify F1r3fly key is same across all networks (uses Bitcoin testnet for derivation)
        if network != NetworkType::Regtest {
            let regtest_keys = WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest)
                .expect("Failed to derive regtest keys");
            assert_eq!(
                keys1.f1r3fly_public_key, regtest_keys.f1r3fly_public_key,
                "F1r3fly key should be network-independent"
            );
        }
    }

    // Verify different mnemonics produce different keys
    let different_mnemonic = generate_mnemonic().expect("Failed to generate different mnemonic");
    let different_keys = WalletKeys::from_mnemonic(&different_mnemonic, NetworkType::Regtest)
        .expect("Failed to derive keys from different mnemonic");
    let original_keys = WalletKeys::from_mnemonic(&mnemonic, NetworkType::Regtest)
        .expect("Failed to derive keys from original mnemonic");

    assert_ne!(
        different_keys.bitcoin_descriptor, original_keys.bitcoin_descriptor,
        "Different mnemonics should produce different Bitcoin descriptors"
    );
    assert_ne!(
        different_keys.f1r3fly_public_key, original_keys.f1r3fly_public_key,
        "Different mnemonics should produce different F1r3fly keys"
    );
}
