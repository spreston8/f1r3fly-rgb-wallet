//! Balance Query Integration Tests
//!
//! Tests RGB balance queries via WalletManager API, focusing on wallet-specific
//! integration rather than re-testing lower-level f1r3fly-rgb functionality.
//!
//! These tests verify:
//! - Balance queries after asset issuance
//! - Empty wallet handling
//! - Single-asset filtering
//! - Error handling for unknown contracts
//!
//! Lower-level seal tracking and balance logic is already tested in f1r3fly-rgb/tests.

use super::{check_f1r3node_available, setup_wallet_with_genesis_utxo};
use crate::common::TestBitcoinEnv;

/// Test balance query immediately after asset issuance
///
/// Verifies:
/// - get_rgb_balance() returns correct data
/// - Genesis UTXO holds full supply
/// - Total balance equals issued supply
/// - UTXO details are correct (txid, vout, amount)
/// - get_occupied_utxos() correctly identifies RGB UTXOs
#[tokio::test]
async fn test_balance_after_issuance() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("balance_after_issuance");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet and issue asset
    let (mut manager, genesis_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    let ticker = "BTC";
    let name = "Bitcoin";
    let supply = 21_000_000u64;
    let precision = 8u8;

    let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: ticker.to_string(),
        name: name.to_string(),
        supply,
        precision,
        genesis_utxo: genesis_utxo.clone(),
    };

    let asset_info = manager
        .issue_asset(request)
        .await
        .expect("Failed to issue asset");

    // Query RGB balance
    let balances = manager
        .get_rgb_balance()
        .await
        .expect("Failed to query RGB balance");

    // Verify we have exactly one asset
    assert_eq!(balances.len(), 1, "Should have exactly one asset");

    let balance = &balances[0];

    // Verify asset metadata
    assert_eq!(balance.ticker, ticker, "Ticker should match");
    assert_eq!(balance.name, name, "Name should match");
    assert_eq!(
        balance.contract_id, asset_info.contract_id,
        "Contract ID should match"
    );

    // Verify total balance equals issued supply
    assert_eq!(
        balance.total, supply,
        "Total balance should equal issued supply"
    );

    // Verify genesis UTXO holds the full supply
    assert_eq!(
        balance.utxo_balances.len(),
        1,
        "Should have exactly one UTXO with balance"
    );

    let utxo_balance = &balance.utxo_balances[0];
    assert_eq!(
        utxo_balance.outpoint, genesis_utxo,
        "Genesis UTXO should hold the balance"
    );
    assert_eq!(
        utxo_balance.amount, supply,
        "Genesis UTXO should hold full supply"
    );

    // Verify get_occupied_utxos() works
    let occupied = manager
        .get_occupied_utxos()
        .await
        .expect("Failed to get occupied UTXOs");

    assert_eq!(occupied.len(), 1, "Should have exactly one occupied UTXO");
    assert_eq!(
        occupied[0].outpoint, genesis_utxo,
        "Occupied UTXO should be genesis UTXO"
    );
    assert_eq!(
        occupied[0].contract_id.as_ref().unwrap(),
        &asset_info.contract_id,
        "Contract ID should match"
    );
    assert_eq!(
        occupied[0].ticker.as_ref().unwrap(),
        ticker,
        "Ticker should match"
    );
    assert_eq!(
        occupied[0].amount.unwrap(),
        supply,
        "Amount should match supply"
    );
}

/// Test balance query on empty wallet
///
/// Verifies:
/// - get_rgb_balance() returns empty vector for wallet with no assets
/// - No errors on empty state
#[tokio::test]
async fn test_balance_empty_wallet() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("balance_empty");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet WITHOUT issuing any assets
    let (mut manager, _genesis_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Query balance on empty wallet
    let balances = manager
        .get_rgb_balance()
        .await
        .expect("Failed to query RGB balance");

    // Verify empty result
    assert_eq!(balances.len(), 0, "Empty wallet should have no balances");
    // Verify occupied UTXOs is also empty
    let occupied = manager
        .get_occupied_utxos()
        .await
        .expect("Failed to get occupied UTXOs");

    assert_eq!(
        occupied.len(),
        0,
        "Empty wallet should have no occupied UTXOs"
    );
}

/// Test querying balance for specific asset when multiple assets exist
///
/// Verifies:
/// - get_asset_balance() returns only the requested asset
/// - Other assets are not included in the result
/// - Balance data is correct
#[tokio::test]
async fn test_balance_specific_asset() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("balance_specific_asset");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet with first asset
    let (mut manager, genesis_utxo_1) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Issue first asset
    let request_1 = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "USD".to_string(),
        name: "US Dollar".to_string(),
        supply: 1_000_000,
        precision: 2,
        genesis_utxo: genesis_utxo_1.clone(),
    };

    let asset_1 = manager
        .issue_asset(request_1)
        .await
        .expect("Failed to issue first asset");

    // Create second genesis UTXO
    let amount_sats = 1_000_000u64;
    let fee_config = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();
    let genesis_result_2 = manager
        .create_utxo(amount_sats, &fee_config, false)
        .expect("Failed to create second UTXO");
    let genesis_txid_2 = genesis_result_2.txid.clone();

    env.wait_for_confirmation(&genesis_txid_2, 1)
        .await
        .expect("Failed to wait for confirmation");

    // Sync with retry to ensure second UTXO is visible (critical for parallel tests)
    for attempt in 1..=5 {
        manager.sync_wallet().expect("Failed to sync wallet");

        // Check if the second UTXO is now visible
        let utxos: Vec<_> = manager
            .bitcoin_wallet()
            .expect("Bitcoin wallet not loaded")
            .inner()
            .list_unspent()
            .collect();
        let utxo_found = utxos
            .iter()
            .any(|utxo| utxo.outpoint.txid.to_string() == genesis_txid_2);

        if utxo_found {
            println!("✅ Second UTXO visible after {} sync attempt(s)", attempt);
            break;
        }

        if attempt < 5 {
            println!(
                "⏳ Second UTXO not visible yet, waiting 1s... (attempt {} /5)",
                attempt + 1
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }

    let genesis_utxo_2 = format!(
        "{}:{}",
        genesis_result_2.txid, genesis_result_2.outpoint.vout
    );

    // Issue second asset
    let request_2 = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "EUR".to_string(),
        name: "Euro".to_string(),
        supply: 2_000_000,
        precision: 2,
        genesis_utxo: genesis_utxo_2.clone(),
    };

    let asset_2 = manager
        .issue_asset(request_2)
        .await
        .expect("Failed to issue second asset");

    // Query balance for first asset specifically
    let balance_1 = manager
        .get_asset_balance(&asset_1.contract_id)
        .await
        .expect("Failed to query balance for first asset");

    // Verify it's the correct asset
    assert_eq!(
        balance_1.contract_id, asset_1.contract_id,
        "Contract ID should match first asset"
    );
    assert_eq!(balance_1.ticker, "USD", "Ticker should be USD");
    assert_eq!(balance_1.name, "US Dollar", "Name should match");
    assert_eq!(balance_1.total, 1_000_000, "Total should match supply");
    assert_eq!(
        balance_1.utxo_balances.len(),
        1,
        "Should have one UTXO balance"
    );

    // Query balance for second asset specifically
    let balance_2 = manager
        .get_asset_balance(&asset_2.contract_id)
        .await
        .expect("Failed to query balance for second asset");

    // Verify it's the correct asset
    assert_eq!(
        balance_2.contract_id, asset_2.contract_id,
        "Contract ID should match second asset"
    );
    assert_eq!(balance_2.ticker, "EUR", "Ticker should be EUR");
    assert_eq!(balance_2.name, "Euro", "Name should match");
    assert_eq!(balance_2.total, 2_000_000, "Total should match supply");
    assert_eq!(
        balance_2.utxo_balances.len(),
        1,
        "Should have one UTXO balance"
    );

    // Verify get_rgb_balance() returns both assets
    let all_balances = manager
        .get_rgb_balance()
        .await
        .expect("Failed to query all balances");

    assert_eq!(
        all_balances.len(),
        2,
        "Should have exactly two assets in total balance"
    );

    // Find each asset in the results
    let has_usd = all_balances.iter().any(|b| b.ticker == "USD");
    let has_eur = all_balances.iter().any(|b| b.ticker == "EUR");

    assert!(has_usd, "Should find USD in total balance");
    assert!(has_eur, "Should find EUR in total balance");
}

/// Test balance query for unknown contract
///
/// Verifies:
/// - get_asset_balance() returns error for non-existent contract
/// - Error message is appropriate
#[tokio::test]
async fn test_balance_unknown_contract() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("balance_unknown_contract");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet and issue one asset
    let (mut manager, genesis_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "BTC".to_string(),
        name: "Bitcoin".to_string(),
        supply: 21_000_000,
        precision: 8,
        genesis_utxo,
    };

    let _asset_info = manager
        .issue_asset(request)
        .await
        .expect("Failed to issue asset");

    // Try to query balance for fake contract ID
    let fake_contract_id = "contract:fakeFake-fakeFake-fakeFake-fakeFake-fakeFake-fakeFake";

    let result = manager.get_asset_balance(fake_contract_id).await;

    // Verify it returns an error
    assert!(result.is_err(), "Should return error for unknown contract");

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Verify error message mentions contract not found or invalid
    assert!(
        error_msg.contains("not found") || error_msg.contains("Invalid"),
        "Error message should indicate contract not found or invalid: {}",
        error_msg
    );
}
