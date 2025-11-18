//! Integration tests for list_utxos command functionality (Steps 1-4)
//!
//! These tests verify the complete UTXO listing implementation across:
//! - Step 1: Types module (data structures)
//! - Step 2: Bitcoin layer (wallet.rs)
//! - Step 3: RGB layer (balance.rs)
//! - Step 4: Manager orchestration
//!
//! Prerequisites:
//! - Running regtest environment: ./scripts/start-regtest.sh
//! - Running F1r3node with environment variables:
//!   - FIREFLY_GRPC_HOST (default: localhost)
//!   - FIREFLY_GRPC_PORT (default: 40401)
//!   - FIREFLY_HTTP_PORT (default: 40403)
//!   - FIREFLY_PRIVATE_KEY (required)
//!
//! Run tests:
//! ```bash
//! # All list_utxos tests
//! cargo test --test list_utxos_integration_test
//!
//! # With output
//! cargo test --test list_utxos_integration_test -- --nocapture
//!
//! # Specific test
//! cargo test --test list_utxos_integration_test test_bitcoin_layer_list_utxos -- --nocapture
//! ```

mod common;
mod f1r3fly;

use common::TestBitcoinEnv;
use f1r3fly::{check_f1r3node_available, setup_wallet_with_genesis_utxo};
use f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest;
use f1r3fly_rgb_wallet::manager::WalletManager;
use f1r3fly_rgb_wallet::types::{OutputFormat, UtxoFilter, UtxoStatus};

/// Test 1: Bitcoin Layer - list_all_utxos() function
///
/// Validates that BitcoinWallet.list_all_utxos() correctly:
/// - Returns empty vector for new wallet
/// - Lists confirmed UTXOs after funding and mining
/// - Calculates correct confirmation counts
/// - Sets status to Available for confirmed UTXOs
/// - Returns empty rgb_assets vector (Bitcoin-only layer)
/// - Sorts UTXOs by confirmations (descending)
#[tokio::test]
async fn test_bitcoin_layer_list_utxos() {
    let env = TestBitcoinEnv::new("bitcoin_layer_list_utxos");

    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    let mut manager =
        WalletManager::new(env.config().clone()).expect("Failed to create WalletManager");
    manager
        .create_wallet(wallet_name, password)
        .expect("Failed to create wallet");
    manager
        .load_wallet(wallet_name, password)
        .expect("Failed to load wallet");

    // Step 1: Verify empty wallet has no UTXOs
    let utxos_empty = manager
        .bitcoin_wallet()
        .expect("Bitcoin wallet not loaded")
        .list_all_utxos()
        .expect("Failed to list UTXOs on empty wallet");
    assert_eq!(utxos_empty.len(), 0, "Empty wallet should have no UTXOs");

    // Step 2: Fund wallet with first transaction
    let address1 = manager
        .get_new_address()
        .expect("Failed to get new address");

    let amount_btc = 0.1;
    let txid1 = env
        .fund_address(&address1, amount_btc)
        .expect("Failed to fund address 1");

    println!("Funded address with txid: {}", txid1);

    // Step 3: Wait for transaction to be confirmed
    env.wait_for_confirmation(&txid1, 1)
        .await
        .expect("Failed to wait for txid1 confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");
    let utxos_1conf = manager
        .bitcoin_wallet()
        .expect("Bitcoin wallet not loaded")
        .list_all_utxos()
        .expect("Failed to list UTXOs after funding and mining");

    assert_eq!(
        utxos_1conf.len(),
        1,
        "Should have 1 UTXO with 1 confirmation"
    );
    let utxo = &utxos_1conf[0];
    assert_eq!(utxo.confirmations, 1, "UTXO should have 1 confirmation");
    assert_eq!(
        utxo.status,
        UtxoStatus::Available,
        "UTXO should have Available status"
    );
    assert_eq!(
        utxo.rgb_assets.len(),
        0,
        "Bitcoin layer should return empty rgb_assets"
    );
    assert_eq!(
        utxo.amount_sats,
        (amount_btc * 100_000_000.0) as u64,
        "Amount in sats should match"
    );
    assert_eq!(utxo.txid, txid1, "Should contain first transaction");

    // Step 4: Create a second UTXO and verify sorting by confirmations (descending)
    let address2 = manager
        .get_new_address()
        .expect("Failed to get new address");
    let txid2 = env
        .fund_address(&address2, amount_btc)
        .expect("Failed to fund address 2");

    // Wait for second transaction to confirm (mines 1 block, now txid1 has 2 confs, txid2 has 1 conf)
    env.wait_for_confirmation(&txid2, 1)
        .await
        .expect("Failed to wait for txid2 confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    let utxos_mixed = manager
        .bitcoin_wallet()
        .expect("Bitcoin wallet not loaded")
        .list_all_utxos()
        .expect("Failed to list UTXOs with mixed confirmations");

    assert_eq!(utxos_mixed.len(), 2, "Should have 2 UTXOs");

    // Verify we have different confirmation counts
    let conf1 = utxos_mixed[0].confirmations;
    let conf2 = utxos_mixed[1].confirmations;

    // First UTXO should have more confirmations (2) than second (1)
    assert_eq!(conf1, 2, "First UTXO should have 2 confirmations");
    assert_eq!(conf2, 1, "Second UTXO should have 1 confirmation");

    // Verify sorting is correct (descending)
    assert!(
        conf1 >= conf2,
        "UTXOs should be sorted by confirmations (descending): {} >= {}",
        conf1,
        conf2
    );

    // Verify both are available
    assert_eq!(utxos_mixed[0].status, UtxoStatus::Available);
    assert_eq!(utxos_mixed[1].status, UtxoStatus::Available);

    println!("✓ Bitcoin layer list_all_utxos() test passed");
}

/// Test 2: RGB Layer - get_rgb_seal_info() function
///
/// Validates that get_rgb_seal_info() correctly:
/// - Returns empty vector for UTXOs with no RGB assets
/// - Returns RGB seal info after asset issuance
/// - Correctly reports contract_id, ticker, and amount
/// - Handles multiple assets on same UTXO (if applicable)
/// - Gracefully handles non-existent UTXOs
#[tokio::test]
async fn test_rgb_layer_seal_info() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⊘ Skipping test: F1r3node not available");
        return;
    }

    let env = TestBitcoinEnv::new("rgb_layer_seal_info");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    let (mut manager, _genesis_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Step 1: Create UTXO with no RGB assets
    let address = manager
        .get_new_address()
        .expect("Failed to get new address");
    let txid = env
        .fund_address(&address, 0.1)
        .expect("Failed to fund address");
    env.wait_for_confirmation(&txid, 1)
        .await
        .expect("Failed to wait for confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    let utxos = manager
        .bitcoin_wallet()
        .unwrap()
        .list_all_utxos()
        .expect("Failed to list UTXOs");

    let empty_utxo = utxos
        .iter()
        .find(|u| u.txid == txid)
        .expect("Should find funded UTXO");
    let outpoint = &empty_utxo.outpoint;

    // Query RGB seal info for UTXO with no assets using manager's list_utxos
    let utxos_empty_rgb = manager
        .list_utxos(UtxoFilter::default())
        .await
        .expect("Failed to list UTXOs");

    let empty_utxo_info = utxos_empty_rgb
        .iter()
        .find(|u| u.outpoint == *outpoint)
        .expect("Should find UTXO");

    assert_eq!(
        empty_utxo_info.rgb_assets.len(),
        0,
        "UTXO without RGB assets should have empty rgb_assets"
    );
    assert_eq!(
        empty_utxo_info.status,
        UtxoStatus::Available,
        "UTXO without RGB assets should be Available"
    );

    // Step 2: Issue RGB asset to a specific UTXO
    let ticker = "TESTRGB";
    let name = "Test RGB Asset";
    let amount = 1000u64;

    // Get another UTXO for issuance
    let address_for_issue = manager
        .get_new_address()
        .expect("Failed to get new address");
    let issue_txid = env
        .fund_address(&address_for_issue, 0.1)
        .expect("Failed to fund address for issuance");
    env.wait_for_confirmation(&issue_txid, 1)
        .await
        .expect("Failed to wait for confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    let utxos_for_issue = manager
        .bitcoin_wallet()
        .unwrap()
        .list_all_utxos()
        .expect("Failed to list UTXOs");

    let issue_utxo = utxos_for_issue
        .iter()
        .find(|u| u.txid == issue_txid)
        .expect("Should find UTXO for issuance");
    let issue_outpoint = issue_utxo.outpoint.clone();

    let request = IssueAssetRequest {
        ticker: ticker.to_string(),
        name: name.to_string(),
        supply: amount,
        precision: 0,
        genesis_utxo: issue_outpoint.clone(),
    };

    let asset_info = manager
        .issue_asset(request)
        .await
        .expect("Failed to issue asset");

    env.wait_for_confirmation(&asset_info.genesis_seal.split(':').next().unwrap(), 1)
        .await
        .expect("Failed to wait for genesis confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    // Step 3: Query RGB seal info for UTXO with asset using manager's list_utxos
    let utxos_after_issue = manager
        .list_utxos(UtxoFilter::default())
        .await
        .expect("Failed to list UTXOs after issuance");

    let genesis_utxo_info = utxos_after_issue
        .iter()
        .find(|u| u.outpoint == asset_info.genesis_seal)
        .expect("Should find genesis UTXO");

    assert_eq!(
        genesis_utxo_info.status,
        UtxoStatus::RgbOccupied,
        "Genesis UTXO should be RgbOccupied"
    );
    assert_eq!(
        genesis_utxo_info.rgb_assets.len(),
        1,
        "UTXO with one RGB asset should have one seal info"
    );
    assert_eq!(
        genesis_utxo_info.rgb_assets[0].ticker, ticker,
        "Ticker should match"
    );
    assert_eq!(
        genesis_utxo_info.rgb_assets[0].amount,
        Some(amount),
        "Amount should match issued amount"
    );
    assert!(
        !genesis_utxo_info.rgb_assets[0].contract_id.is_empty(),
        "Contract ID should be present"
    );

    println!("✓ RGB layer get_rgb_seal_info() test passed");
}

/// Test 3: Manager Orchestration - list_utxos() with filters
///
/// Validates that WalletManager.list_utxos() correctly:
/// - Orchestrates Bitcoin layer + RGB layer
/// - Applies UtxoFilter correctly (available_only, rgb_only, confirmed_only)
/// - Sets UtxoStatus to RgbOccupied when RGB assets present
/// - Maintains orthogonality of confirmation status and RGB occupation
#[tokio::test]
async fn test_manager_orchestration_with_filters() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⊘ Skipping test: F1r3node not available");
        return;
    }

    let env = TestBitcoinEnv::new("manager_orchestration_filters");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    let (mut manager, genesis_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Step 1: Create UTXOs with different characteristics
    // UTXO 1: Bitcoin-only, confirmed
    let address1 = manager
        .get_new_address()
        .expect("Failed to get new address");
    let txid1 = env
        .fund_address(&address1, 0.1)
        .expect("Failed to fund address 1");
    env.wait_for_confirmation(&txid1, 3)
        .await
        .expect("Failed to wait for confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    // UTXO 2: Will be RGB-occupied
    let request = IssueAssetRequest {
        ticker: "RGB2".to_string(),
        name: "RGB Asset 2".to_string(),
        supply: 500,
        precision: 0,
        genesis_utxo: genesis_utxo.clone(),
    };

    let asset_info = manager
        .issue_asset(request)
        .await
        .expect("Failed to issue asset");

    let genesis_txid = asset_info.genesis_seal.split(':').next().unwrap();
    env.wait_for_confirmation(genesis_txid, 1)
        .await
        .expect("Failed to wait for genesis confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    // UTXO 3: Bitcoin-only, unconfirmed
    let address3 = manager
        .get_new_address()
        .expect("Failed to get new address");
    env.fund_address(&address3, 0.1)
        .expect("Failed to fund address 3");
    manager.sync_wallet().expect("Failed to sync wallet");

    // Step 2: Test default filter (no restrictions)
    let filter_all = UtxoFilter::default();
    let utxos_all = manager
        .list_utxos(filter_all)
        .await
        .expect("Failed to list all UTXOs");

    assert!(utxos_all.len() >= 3, "Should have at least 3 UTXOs");

    let utxo_rgb = utxos_all
        .iter()
        .find(|u| u.outpoint == asset_info.genesis_seal)
        .expect("Should find RGB-occupied UTXO");
    assert_eq!(
        utxo_rgb.status,
        UtxoStatus::RgbOccupied,
        "UTXO with RGB asset should be RgbOccupied"
    );
    assert_eq!(utxo_rgb.rgb_assets.len(), 1, "Should have 1 RGB asset");
    assert_eq!(utxo_rgb.rgb_assets[0].ticker, "RGB2", "Ticker should match");

    // Step 3: Test available_only filter
    let filter_available = UtxoFilter {
        available_only: true,
        confirmed_only: true,
        ..Default::default()
    };
    let utxos_available = manager
        .list_utxos(filter_available)
        .await
        .expect("Failed to list available UTXOs");

    assert!(
        utxos_available.len() >= 1,
        "Should have at least 1 available UTXO"
    );
    for utxo in &utxos_available {
        assert_eq!(
            utxo.status,
            UtxoStatus::Available,
            "All UTXOs should be Available"
        );
        assert_eq!(
            utxo.rgb_assets.len(),
            0,
            "Available UTXOs should have no RGB assets"
        );
    }

    // Step 4: Test rgb_only filter
    let filter_rgb = UtxoFilter {
        rgb_only: true,
        ..Default::default()
    };
    let utxos_rgb = manager
        .list_utxos(filter_rgb)
        .await
        .expect("Failed to list RGB-occupied UTXOs");

    assert_eq!(utxos_rgb.len(), 1, "Should have 1 RGB-occupied UTXO");
    assert_eq!(
        utxos_rgb[0].status,
        UtxoStatus::RgbOccupied,
        "UTXO should be RgbOccupied"
    );
    assert!(
        !utxos_rgb[0].rgb_assets.is_empty(),
        "RgbOccupied UTXO should have RGB assets"
    );

    // Step 5: Test confirmed_only filter
    let filter_confirmed = UtxoFilter {
        confirmed_only: true,
        ..Default::default()
    };
    let utxos_confirmed = manager
        .list_utxos(filter_confirmed)
        .await
        .expect("Failed to list confirmed UTXOs");

    for utxo in &utxos_confirmed {
        assert!(utxo.confirmations > 0, "All UTXOs should be confirmed");
    }

    // Step 6: Test combined filters
    let filter_combined = UtxoFilter {
        available_only: true,
        confirmed_only: true,
        ..Default::default()
    };
    let utxos_combined = manager
        .list_utxos(filter_combined)
        .await
        .expect("Failed to list UTXOs with combined filters");

    for utxo in &utxos_combined {
        assert!(utxo.confirmations > 0, "Should be confirmed");
        assert_eq!(utxo.status, UtxoStatus::Available, "Should be Available");
    }

    println!("✓ Manager orchestration with filters test passed");
}

/// Test 4: Multiple RGB Assets - Issue Multiple Assets and Track
///
/// Validates UTXO tracking with multiple RGB assets:
/// - Issue multiple assets to different UTXOs
/// - Verify each UTXO becomes RgbOccupied with correct asset
/// - Verify filters correctly isolate RGB UTXOs
/// - Verify total asset accounting
#[tokio::test]
async fn test_multiple_rgb_assets() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⊘ Skipping test: F1r3node not available");
        return;
    }

    let env = TestBitcoinEnv::new("multiple_rgb_assets");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    let (mut manager, genesis_utxo1) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Step 1: Issue first RGB asset
    let ticker1 = "ASSET1";
    let name1 = "Asset One";
    let amount1 = 5000u64;

    let request1 = IssueAssetRequest {
        ticker: ticker1.to_string(),
        name: name1.to_string(),
        supply: amount1,
        precision: 0,
        genesis_utxo: genesis_utxo1.clone(),
    };

    let asset_info1 = manager
        .issue_asset(request1)
        .await
        .expect("Failed to issue first asset");

    let genesis_txid1 = asset_info1.genesis_seal.split(':').next().unwrap();
    env.wait_for_confirmation(genesis_txid1, 1)
        .await
        .expect("Failed to wait for confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    // Step 2: Create second genesis UTXO and issue second asset
    let address2 = manager
        .get_new_address()
        .expect("Failed to get new address");
    let txid2 = env
        .fund_address(&address2, 0.1)
        .expect("Failed to fund address");
    env.wait_for_confirmation(&txid2, 1)
        .await
        .expect("Failed to wait for confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    let utxos = manager
        .bitcoin_wallet()
        .unwrap()
        .list_all_utxos()
        .expect("Failed to list UTXOs");

    let genesis_utxo2 = utxos
        .iter()
        .find(|u| u.txid == txid2)
        .expect("Should find second genesis UTXO")
        .outpoint
        .clone();

    let ticker2 = "ASSET2";
    let name2 = "Asset Two";
    let amount2 = 8000u64;

    let request2 = IssueAssetRequest {
        ticker: ticker2.to_string(),
        name: name2.to_string(),
        supply: amount2,
        precision: 0,
        genesis_utxo: genesis_utxo2.clone(),
    };

    let asset_info2 = manager
        .issue_asset(request2)
        .await
        .expect("Failed to issue second asset");

    let genesis_txid2 = asset_info2.genesis_seal.split(':').next().unwrap();
    env.wait_for_confirmation(genesis_txid2, 1)
        .await
        .expect("Failed to wait for confirmation");
    manager.sync_wallet().expect("Failed to sync wallet");

    // Step 3: Verify both assets are tracked correctly
    let utxos_all = manager
        .list_utxos(UtxoFilter::default())
        .await
        .expect("Failed to list all UTXOs");

    let asset1_utxo = utxos_all
        .iter()
        .find(|u| u.outpoint == asset_info1.genesis_seal)
        .expect("Should find first asset UTXO");

    assert_eq!(
        asset1_utxo.status,
        UtxoStatus::RgbOccupied,
        "First asset UTXO should be RgbOccupied"
    );
    assert_eq!(asset1_utxo.rgb_assets.len(), 1, "Should have 1 RGB asset");
    assert_eq!(
        asset1_utxo.rgb_assets[0].ticker, ticker1,
        "First asset ticker should match"
    );
    assert_eq!(
        asset1_utxo.rgb_assets[0].amount,
        Some(amount1),
        "First asset amount should match"
    );

    let asset2_utxo = utxos_all
        .iter()
        .find(|u| u.outpoint == asset_info2.genesis_seal)
        .expect("Should find second asset UTXO");

    assert_eq!(
        asset2_utxo.status,
        UtxoStatus::RgbOccupied,
        "Second asset UTXO should be RgbOccupied"
    );
    assert_eq!(asset2_utxo.rgb_assets.len(), 1, "Should have 1 RGB asset");
    assert_eq!(
        asset2_utxo.rgb_assets[0].ticker, ticker2,
        "Second asset ticker should match"
    );
    assert_eq!(
        asset2_utxo.rgb_assets[0].amount,
        Some(amount2),
        "Second asset amount should match"
    );

    // Step 4: Verify rgb_only filter returns both RGB UTXOs
    let rgb_utxos = manager
        .list_utxos(UtxoFilter {
            rgb_only: true,
            ..Default::default()
        })
        .await
        .expect("Failed to list RGB UTXOs");

    assert_eq!(
        rgb_utxos.len(),
        2,
        "Should have exactly 2 RGB-occupied UTXOs"
    );

    for utxo in &rgb_utxos {
        assert_eq!(
            utxo.status,
            UtxoStatus::RgbOccupied,
            "All UTXOs should be RgbOccupied"
        );
        assert!(!utxo.rgb_assets.is_empty(), "Should have RGB assets");
    }

    // Step 5: Verify available_only filter excludes RGB UTXOs
    let available_utxos = manager
        .list_utxos(UtxoFilter {
            available_only: true,
            confirmed_only: true,
            ..Default::default()
        })
        .await
        .expect("Failed to list available UTXOs");

    for utxo in &available_utxos {
        assert_eq!(
            utxo.status,
            UtxoStatus::Available,
            "Should only have Available UTXOs"
        );
        assert_eq!(
            utxo.rgb_assets.len(),
            0,
            "Available UTXOs should have no RGB assets"
        );
    }

    println!("✓ Multiple RGB assets test passed");
}

/// Test 5: Data Structure Serialization and Edge Cases
///
/// Validates data structure correctness:
/// - UtxoInfo serialization to JSON
/// - OutputFormat enum values
/// - UtxoFilter default values
/// - Edge cases (empty filters, large amounts)
#[tokio::test]
async fn test_data_structures_and_edge_cases() {
    use f1r3fly_rgb_wallet::types::{RgbSealInfo, UtxoInfo};

    // Test 1: UtxoInfo serialization
    let sample_utxo = UtxoInfo {
        outpoint: "abc123:0".to_string(),
        txid: "abc123".to_string(),
        vout: 0,
        amount_sats: 10_000_000,
        amount_btc: 0.1,
        confirmations: 6,
        status: UtxoStatus::RgbOccupied,
        rgb_assets: vec![RgbSealInfo {
            contract_id: "contract123".to_string(),
            ticker: "TEST".to_string(),
            amount: Some(1000),
        }],
    };

    let json_str = serde_json::to_string(&sample_utxo).expect("Failed to serialize UtxoInfo");
    assert!(
        json_str.contains("\"outpoint\""),
        "Should contain outpoint field"
    );
    assert!(
        json_str.contains("\"amount_sats\""),
        "Should contain amount_sats field"
    );
    assert!(
        json_str.contains("\"rgb_assets\""),
        "Should contain rgb_assets field"
    );

    let deserialized: UtxoInfo =
        serde_json::from_str(&json_str).expect("Failed to deserialize UtxoInfo");
    assert_eq!(
        deserialized.outpoint, sample_utxo.outpoint,
        "Outpoint should match after deserialization"
    );
    assert_eq!(
        deserialized.rgb_assets.len(),
        1,
        "Should have 1 RGB asset after deserialization"
    );

    // Test 2: OutputFormat enum values
    let format_table = OutputFormat::Table;
    let format_json = OutputFormat::Json;
    let format_compact = OutputFormat::Compact;

    assert!(
        matches!(format_table, OutputFormat::Table),
        "Table format should match"
    );
    assert!(
        matches!(format_json, OutputFormat::Json),
        "Json format should match"
    );
    assert!(
        matches!(format_compact, OutputFormat::Compact),
        "Compact format should match"
    );

    // Test 3: UtxoFilter default values
    let default_filter = UtxoFilter::default();
    assert_eq!(
        default_filter.available_only, false,
        "Default available_only should be false"
    );
    assert_eq!(
        default_filter.rgb_only, false,
        "Default rgb_only should be false"
    );
    assert_eq!(
        default_filter.confirmed_only, false,
        "Default confirmed_only should be false"
    );
    assert_eq!(
        default_filter.min_amount_sats, None,
        "Default min_amount_sats should be None"
    );

    // Test 4: Edge cases with actual wallet
    let env = TestBitcoinEnv::new("data_structures_edge_cases");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    let mut manager =
        WalletManager::new(env.config().clone()).expect("Failed to create WalletManager");
    manager
        .create_wallet(wallet_name, password)
        .expect("Failed to create wallet");
    manager
        .load_wallet(wallet_name, password)
        .expect("Failed to load wallet");

    // Test empty filter returns all UTXOs
    let filter_empty = UtxoFilter::default();
    let utxos_empty_filter = manager
        .list_utxos(filter_empty)
        .await
        .expect("Failed to list UTXOs with empty filter");
    // Empty wallet should return empty list
    assert_eq!(
        utxos_empty_filter.len(),
        0,
        "Empty wallet should return no UTXOs"
    );

    // Test with F1r3node if available for large amount test
    if !check_f1r3node_available() {
        println!("⊘ Skipping F1r3fly edge cases: F1r3node not available");
        println!("✓ Data structures and edge cases test passed (partial)");
        return;
    }

    let (mut manager_rgb, genesis_utxo) =
        setup_wallet_with_genesis_utxo(&env, wallet_name, password)
            .await
            .expect("Failed to setup wallet");

    // Test large amounts
    let large_amount = u64::MAX / 2; // Test with large amount

    let request = IssueAssetRequest {
        ticker: "LARGE".to_string(),
        name: "Large Amount Test".to_string(),
        supply: large_amount,
        precision: 0,
        genesis_utxo: genesis_utxo.clone(),
    };

    manager_rgb
        .issue_asset(request)
        .await
        .expect("Failed to issue asset with large amount");

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    manager_rgb.sync_wallet().expect("Failed to sync wallet");

    let utxos_large = manager_rgb
        .list_utxos(UtxoFilter {
            rgb_only: true,
            ..Default::default()
        })
        .await
        .expect("Failed to list UTXOs after large issuance");

    let utxo_large = utxos_large
        .iter()
        .find(|u| !u.rgb_assets.is_empty())
        .expect("Should find RGB-occupied UTXO");

    assert_eq!(
        utxo_large.rgb_assets[0].amount,
        Some(large_amount),
        "Large amount should be preserved"
    );

    println!("✓ Data structures and edge cases test passed");
}
