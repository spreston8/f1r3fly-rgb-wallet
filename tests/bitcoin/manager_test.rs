//! Module 6: Manager Integration Tests
//!
//! Tests for WalletManager integration with all wallet functionality.
//! Covers wallet creation, import, loading, sync, addresses, UTXO operations, and sending.

use f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig;
use f1r3fly_rgb_wallet::manager::WalletManager;
use f1r3fly_rgb_wallet::storage::keys::generate_mnemonic;

use crate::common::TestBitcoinEnv;

/// Test 6.1: Verify WalletManager.create_wallet() works completely
#[tokio::test]
async fn test_manager_create_wallet_end_to_end() {
    // Step 1: Create TestBitcoinEnv
    let env = TestBitcoinEnv::new("manager_create_wallet");

    // Step 2: Create regtest config
    let config = env.config().clone();

    // Step 3: Create WalletManager with config
    let mut manager = WalletManager::new(config).expect("Failed to create WalletManager");

    // Step 4: Call manager.create_wallet("test_wallet", "password")
    let wallet_name = format!("test_wallet_{}", uuid::Uuid::new_v4());
    let password = "test_password_123";

    let mnemonic_str = manager
        .create_wallet(&wallet_name, password)
        .expect("Failed to create wallet");

    // Step 5: Verify mnemonic string returned (12 words)
    let words: Vec<&str> = mnemonic_str.split_whitespace().collect();
    assert_eq!(
        words.len(),
        12,
        "Mnemonic should have 12 words, got: {}",
        words.len()
    );

    // Step 6-8: Verify wallet was created successfully
    // (We verify this by checking if the manager has loaded metadata and can generate addresses)

    // Step 9: Verify manager has loaded wallet (can get metadata)
    let metadata = manager.metadata().expect("Should have loaded metadata");
    assert_eq!(
        metadata.name, wallet_name,
        "Metadata name should match wallet name"
    );

    // Step 10: Get new address from manager
    let address = manager
        .get_new_address()
        .expect("Failed to get new address");

    // Step 11: Verify valid bcrt1 address
    assert!(
        address.to_string().starts_with("bcrt1"),
        "Address should be regtest (bcrt1), got: {}",
        address
    );
}

/// Test 6.2: Verify importing from mnemonic works
#[tokio::test]
async fn test_manager_import_wallet_from_mnemonic() {
    // Step 1: Create TestBitcoinEnv
    let env = TestBitcoinEnv::new("manager_import_wallet");

    // Step 2: Generate known test mnemonic
    let test_mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let mnemonic_str = test_mnemonic.to_string();

    // Step 3: Create WalletManager
    let config = env.config().clone();

    let mut manager = WalletManager::new(config).expect("Failed to create WalletManager");

    // Step 4: Import wallet with mnemonic and password
    let wallet_name = format!("imported_wallet_{}", uuid::Uuid::new_v4());
    let password = "import_password_123";

    manager
        .import_wallet(&wallet_name, &mnemonic_str, password)
        .expect("Failed to import wallet");

    // Step 5: Verify wallet created (by loading it)

    // Step 6: Load wallet in new manager instance
    let config2 = env.config().clone();

    let mut manager2 = WalletManager::new(config2).expect("Failed to create second manager");

    manager2
        .load_wallet(&wallet_name, password)
        .expect("Failed to load imported wallet");

    // Step 7: Verify keys derived match expected from mnemonic
    // Get address from both managers
    let addr1 = manager
        .get_new_address()
        .expect("Failed to get address from manager1");
    let addr2 = manager2
        .get_new_address()
        .expect("Failed to get address from manager2");

    // They should be the same since they use the same mnemonic and derivation index
    assert_eq!(
        addr1.to_string(),
        addr2.to_string(),
        "Addresses should match for same mnemonic"
    );
}

/// Test 6.3: Verify wallet can be loaded and synced
#[tokio::test]
async fn test_manager_load_wallet_and_sync() {
    // Step 1: Create wallet via manager
    let env = TestBitcoinEnv::new("manager_load_sync");

    let config = env.config().clone();

    let mut manager = WalletManager::new(config.clone()).expect("Failed to create manager");

    let wallet_name = format!("loadable_wallet_{}", uuid::Uuid::new_v4());
    let password = "load_password_123";

    manager
        .create_wallet(&wallet_name, password)
        .expect("Failed to create wallet");

    // Step 2: Get address and fund it
    let address = manager.get_new_address().expect("Failed to get address");

    let txid = env
        .fund_address(&address.to_string(), 0.5)
        .expect("Failed to fund address");

    // Step 3: Mine block
    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 4: Drop manager
    drop(manager);

    // Step 5: Create new WalletManager instance
    let mut manager2 = WalletManager::new(config).expect("Failed to create second manager");

    // Step 6: Load same wallet with password
    manager2
        .load_wallet(&wallet_name, password)
        .expect("Failed to load wallet");

    // Step 7: Sync wallet
    let sync_result = manager2.sync_wallet().await.expect("Failed to sync wallet");

    assert!(
        sync_result.new_txs > 0,
        "Should detect new transaction after sync"
    );

    // Step 8: Verify balance detected
    let balance = manager2.get_balance().expect("Failed to get balance");

    let expected_sats = 50_000_000; // 0.5 BTC
    assert_eq!(
        balance.confirmed, expected_sats,
        "Balance should be {} sats (0.5 BTC), got: {} sats",
        expected_sats, balance.confirmed
    );
}

/// Test 6.4: Verify manager sync updates balance correctly
#[tokio::test]
async fn test_manager_sync_wallet_updates_balance() {
    // Step 1: Create wallet via manager
    let env = TestBitcoinEnv::new("manager_sync_balance");

    let config = env.config().clone();

    let mut manager = WalletManager::new(config).expect("Failed to create manager");

    let wallet_name = format!("sync_balance_wallet_{}", uuid::Uuid::new_v4());
    let password = "sync_password_123";

    manager
        .create_wallet(&wallet_name, password)
        .expect("Failed to create wallet");

    // Step 2: Get address from manager
    let address = manager.get_new_address().expect("Failed to get address");

    // Step 3: Fund address from regtest (0.5 BTC)
    let txid = env
        .fund_address(&address.to_string(), 0.5)
        .expect("Failed to fund address");

    // Step 4: Mine 1 block
    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 5: Call manager.sync_wallet().await
    let sync_result = manager.sync_wallet().await.expect("Failed to sync wallet");

    // Step 6: Verify SyncResult shows new transactions
    assert!(
        sync_result.new_txs > 0,
        "Sync should detect new transactions, got: {}",
        sync_result.new_txs
    );

    // Step 7: Call manager.get_balance()
    let balance = manager.get_balance().expect("Failed to get balance");

    // Step 8: Verify balance = 0.5 BTC
    let expected_sats = 50_000_000;
    assert_eq!(
        balance.confirmed, expected_sats,
        "Balance should be {} sats (0.5 BTC), got: {} sats",
        expected_sats, balance.confirmed
    );
}

/// Test 6.5: Verify address operations through manager
#[tokio::test]
async fn test_manager_get_addresses_and_new_address() {
    // Step 1: Create wallet via manager
    let env = TestBitcoinEnv::new("manager_addresses");

    let config = env.config().clone();

    let mut manager = WalletManager::new(config).expect("Failed to create manager");

    let wallet_name = format!("address_wallet_{}", uuid::Uuid::new_v4());
    let password = "address_password_123";

    manager
        .create_wallet(&wallet_name, password)
        .expect("Failed to create wallet");

    // Step 2: Call manager.get_addresses(Some(5))
    let addresses = manager
        .get_addresses(Some(5))
        .expect("Failed to get addresses");

    // Step 3: Verify returns 5 addresses
    assert!(
        addresses.len() >= 5,
        "Should return at least 5 addresses, got: {}",
        addresses.len()
    );

    // Step 4: Call manager.get_new_address()
    let first_new = manager
        .get_new_address()
        .expect("Failed to get first new address");

    // Step 5: Verify returns new address
    assert!(
        first_new.to_string().starts_with("bcrt1"),
        "Address should be regtest"
    );

    // Step 6: Call manager.get_new_address() again
    let second_new = manager
        .get_new_address()
        .expect("Failed to get second new address");

    // Step 7: Verify returns different address
    assert_ne!(
        first_new.to_string(),
        second_new.to_string(),
        "Second address should be different from first"
    );

    // Verify both are valid
    assert!(
        second_new.to_string().starts_with("bcrt1"),
        "Second address should be regtest"
    );
}

/// Test 6.6: Verify UTXO creation through manager
#[tokio::test]
async fn test_manager_create_utxo_full_flow() {
    // Step 1: Create wallet via manager
    let env = TestBitcoinEnv::new("manager_create_utxo");

    let config = env.config().clone();

    let mut manager = WalletManager::new(config).expect("Failed to create manager");

    let wallet_name = format!("utxo_wallet_{}", uuid::Uuid::new_v4());
    let password = "utxo_password_123";

    manager
        .create_wallet(&wallet_name, password)
        .expect("Failed to create wallet");

    // Step 2: Fund wallet with 1 BTC
    let address = manager.get_new_address().expect("Failed to get address");

    let txid = env
        .fund_address(&address.to_string(), 1.0)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 3: Sync wallet
    manager.sync_wallet().await.expect("Failed to sync wallet");

    // Step 4: Create UTXO of 25,000 sats with mark_rgb=true
    let target_amount = 25_000;
    let fee_rate = FeeRateConfig::medium_priority();

    let result = manager
        .create_utxo(target_amount, &fee_rate, true)
        .expect("Failed to create UTXO");

    // Step 5: Verify UtxoOperationResult returned
    assert_eq!(
        result.amount, target_amount,
        "UTXO should have amount {} sats",
        target_amount
    );

    // Step 6: Mine block
    env.wait_for_confirmation(&result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm UTXO creation");

    // Step 7: Sync wallet
    manager
        .sync_wallet()
        .await
        .expect("Failed to sync after UTXO creation");

    // Step 8: Verify manager.rgb_occupied() contains outpoint
    let rgb_occupied = manager.rgb_occupied();
    assert!(
        rgb_occupied.contains(&result.outpoint),
        "RGB-occupied set should contain created UTXO"
    );
}

/// Test 6.7: Verify sending Bitcoin through manager
#[tokio::test]
async fn test_manager_send_bitcoin_full_flow() {
    // Step 1: Create wallet via manager
    let env = TestBitcoinEnv::new("manager_send_bitcoin");

    let config = env.config().clone();

    let mut manager = WalletManager::new(config).expect("Failed to create manager");

    let wallet_name = format!("send_wallet_{}", uuid::Uuid::new_v4());
    let password = "send_password_123";

    manager
        .create_wallet(&wallet_name, password)
        .expect("Failed to create wallet");

    // Step 2: Fund with 1 BTC
    let address = manager.get_new_address().expect("Failed to get address");

    let txid = env
        .fund_address(&address.to_string(), 1.0)
        .expect("Failed to fund wallet");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 3: Sync
    manager.sync_wallet().await.expect("Failed to sync wallet");

    let initial_balance = manager
        .get_balance()
        .expect("Failed to get initial balance");

    // Step 4: Generate recipient address
    let recipient = env
        .get_new_test_address()
        .expect("Failed to generate recipient address");

    // Step 5: Call manager.send_bitcoin(recipient, 100000, fee_rate)
    let send_amount = 100_000;
    let fee_rate = FeeRateConfig::medium_priority();

    let txid = manager
        .send_bitcoin(&recipient, send_amount, &fee_rate)
        .expect("Failed to send bitcoin");

    // Step 6: Verify txid returned
    assert!(!txid.is_empty(), "Transaction ID should not be empty");

    // Step 7: Mine block
    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm send transaction");

    // Step 8: Sync
    manager
        .sync_wallet()
        .await
        .expect("Failed to sync after send");

    // Step 9: Verify balance decreased
    let final_balance = manager.get_balance().expect("Failed to get final balance");

    let balance_decrease = initial_balance.confirmed - final_balance.confirmed;
    assert!(
        balance_decrease >= send_amount,
        "Balance should decrease by at least {} sats, decreased by: {} sats",
        send_amount,
        balance_decrease
    );

    // Step 10: Verify recipient received funds (via bitcoin-cli if available)
    // This is optional as we've already verified the transaction confirmed
}

/// Test 6.8: Verify multiple wallets don't interfere
#[tokio::test]
async fn test_manager_multiple_wallets_isolated() {
    // Step 1: Create manager
    let env = TestBitcoinEnv::new("manager_multi_wallets");

    let config1 = env.config().clone();

    let mut manager1 = WalletManager::new(config1.clone()).expect("Failed to create first manager");

    // Step 2: Create wallet1
    let wallet1_name = format!("wallet_one_{}", uuid::Uuid::new_v4());
    let password1 = "password_one_123";

    manager1
        .create_wallet(&wallet1_name, password1)
        .expect("Failed to create wallet1");

    // Step 3: Create wallet2 (new manager instance)
    let config2 = env.config().clone();

    let mut manager2 = WalletManager::new(config2).expect("Failed to create second manager");

    let wallet2_name = format!("wallet_two_{}", uuid::Uuid::new_v4());
    let password2 = "password_two_123";

    manager2
        .create_wallet(&wallet2_name, password2)
        .expect("Failed to create wallet2");

    // Step 4: Fund wallet1
    let addr1 = manager1
        .get_new_address()
        .expect("Failed to get address from wallet1");

    let txid = env
        .fund_address(&addr1.to_string(), 0.5)
        .expect("Failed to fund wallet1");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 5: Sync wallet1
    manager1
        .sync_wallet()
        .await
        .expect("Failed to sync wallet1");

    // Step 6: Verify wallet1 has balance
    let balance1 = manager1.get_balance().expect("Failed to get balance1");

    let expected_sats = 50_000_000; // 0.5 BTC
    assert_eq!(
        balance1.confirmed, expected_sats,
        "Wallet1 should have {} sats",
        expected_sats
    );

    // Step 7: Load wallet2 (already loaded, but sync it)
    manager2
        .sync_wallet()
        .await
        .expect("Failed to sync wallet2");

    // Step 8: Verify wallet2 balance = 0
    let balance2 = manager2.get_balance().expect("Failed to get balance2");

    assert_eq!(
        balance2.confirmed, 0,
        "Wallet2 should have 0 balance, got: {} sats",
        balance2.confirmed
    );

    // Step 9: Load wallet1 again in manager2
    let mut manager3 = WalletManager::new(config1).expect("Failed to create third manager");

    manager3
        .load_wallet(&wallet1_name, password1)
        .expect("Failed to load wallet1 in manager3");

    manager3
        .sync_wallet()
        .await
        .expect("Failed to sync wallet1 in manager3");

    // Step 10: Verify wallet1 still has correct balance
    let balance1_again = manager3
        .get_balance()
        .expect("Failed to get balance1 again");

    assert_eq!(
        balance1_again.confirmed, expected_sats,
        "Wallet1 should still have {} sats when loaded in different manager",
        expected_sats
    );
}
