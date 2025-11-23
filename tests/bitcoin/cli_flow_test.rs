//! CLI Flow Test
//!
//! This test reproduces the exact flow that the CLI uses to catch any issues
//! that don't appear in the regular integration tests.

use f1r3fly_rgb_wallet::bitcoin::FeeRateConfig;
use f1r3fly_rgb_wallet::manager::WalletManager;

use crate::common::TestBitcoinEnv;

/// Test that reproduces the exact CLI flow:
/// 1. Create wallet via manager
/// 2. Get addresses via manager.get_addresses() (not get_new_address)
/// 3. Fund the first address
/// 4. Sync wallet
/// 5. Try to create UTXO (this is where CLI fails with witness hash mismatch)
#[tokio::test]
async fn test_cli_flow_get_addresses_then_spend() {
    let env = TestBitcoinEnv::new("cli_flow_test");

    let config = env.config().clone();
    let mut manager = WalletManager::new(config).expect("Failed to create manager");

    let wallet_name = format!("cli_test_{}", uuid::Uuid::new_v4());
    let password = "test_password_123";

    // Step 1: Create wallet (like CLI 'wallet create')
    manager
        .create_wallet(&wallet_name, password)
        .expect("Failed to create wallet");

    // Step 2: Sync wallet initially (like CLI 'sync')
    manager.sync_wallet().await.expect("Failed to initial sync");

    // Step 3: Get addresses via get_addresses() with count=1 (like CLI 'get-addresses --count 1')
    let addresses = manager
        .get_addresses(Some(1))
        .expect("Failed to get addresses");

    assert!(!addresses.is_empty(), "Should have at least one address");
    let address = addresses[0].address.to_string();

    println!("Funding address: {}", address);

    // Step 4: Fund the address
    let txid = env
        .fund_address(&address, 1.0)
        .expect("Failed to fund address");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 5: Sync wallet (like CLI 'sync' after funding)
    manager
        .sync_wallet()
        .await
        .expect("Failed to sync after funding");

    // Step 6: Check balance
    let balance = manager.get_balance().expect("Failed to get balance");
    assert!(
        balance.confirmed > 0,
        "Should have confirmed balance after sync, got: {} sats",
        balance.confirmed
    );

    println!("Balance after funding: {} sats", balance.confirmed);

    // Step 7: Try to create UTXO (like CLI 'create-utxo --amount 0.0003')
    // THIS IS WHERE THE CLI FAILS WITH WITNESS HASH MISMATCH
    let target_amount = 30_000; // 0.0003 BTC
    let fee_rate = FeeRateConfig::medium_priority();

    let result = manager
        .create_utxo(target_amount, &fee_rate, false)
        .expect("Failed to create UTXO - witness hash mismatch error");

    println!("✓ Successfully created UTXO: {}", result.txid);

    // Step 8: Verify the UTXO was created
    assert_eq!(
        result.amount, target_amount,
        "UTXO should have correct amount"
    );

    // Step 9: Confirm the transaction
    env.wait_for_confirmation(&result.txid.to_string(), 1)
        .await
        .expect("Failed to confirm UTXO creation");

    // Step 10: Sync and verify balance changed
    manager
        .sync_wallet()
        .await
        .expect("Failed to sync after UTXO creation");

    let final_balance = manager.get_balance().expect("Failed to get final balance");
    assert!(
        final_balance.confirmed < balance.confirmed,
        "Balance should decrease after creating UTXO (due to fees)"
    );

    println!("✓ CLI flow test passed!");
}

/// Test that reproduces the CLI flow with wallet reload:
/// This test creates a wallet, closes it, then reloads it from disk
/// to match the CLI behavior where each command loads the wallet fresh.
#[tokio::test]
async fn test_cli_flow_with_wallet_reload() {
    let env = TestBitcoinEnv::new("cli_flow_reload");

    let config = env.config().clone();

    let wallet_name = format!("cli_reload_{}", uuid::Uuid::new_v4());
    let password = "test_password_123";

    // Step 1: Create wallet and get NEW address (this properly reveals and persists)
    let address = {
        let mut manager = WalletManager::new(config.clone()).expect("Failed to create manager");

        manager
            .create_wallet(&wallet_name, password)
            .expect("Failed to create wallet");

        // Sync
        manager.sync_wallet().await.expect("Failed to sync");

        // Get NEW address - this is the correct way, not get_addresses()
        manager
            .get_new_address()
            .expect("Failed to get new address")
    }; // manager dropped here, wallet closed

    println!("Got address from first session: {}", address);

    // Step 2: Fund the address (wallet is closed)
    let txid = env
        .fund_address(&address, 1.0)
        .expect("Failed to fund address");

    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to confirm funding");

    // Step 3: Reload wallet and sync (like CLI 'sync' command)
    {
        let mut manager = WalletManager::new(config.clone()).expect("Failed to create manager");
        manager
            .load_wallet(&wallet_name, password)
            .expect("Failed to load wallet");

        manager
            .sync_wallet()
            .await
            .expect("Failed to sync after funding");

        let balance = manager.get_balance().expect("Failed to get balance");
        println!("Balance after sync: {} sats", balance.confirmed);

        assert!(balance.confirmed > 0, "Should have funds after sync");
    } // manager dropped

    // Step 4: Reload wallet again and try to spend (like CLI 'create-utxo' command)
    {
        let mut manager = WalletManager::new(config.clone()).expect("Failed to create manager");
        manager
            .load_wallet(&wallet_name, password)
            .expect("Failed to load wallet for spending");

        // Sync before spending (might help with witness issues)
        manager
            .sync_wallet()
            .await
            .expect("Failed to sync before spending");

        let target_amount = 30_000;
        let fee_rate = FeeRateConfig::medium_priority();

        // THIS IS WHERE THE CLI MIGHT FAIL
        let result = manager
            .create_utxo(target_amount, &fee_rate, false)
            .expect("Failed to create UTXO after reload - witness hash mismatch?");

        println!("✓ Successfully created UTXO after reload: {}", result.txid);

        assert_eq!(
            result.amount, target_amount,
            "UTXO should have correct amount"
        );
    }

    println!("✓ CLI flow with wallet reload test passed!");
}
