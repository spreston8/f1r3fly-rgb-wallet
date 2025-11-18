//! Asset Issuance Integration Tests
//!
//! Tests for F1r3fly-RGB asset issuance functionality via WalletManager.
//! These tests verify the complete flow from wallet creation through asset
//! issuance, metadata extraction, and state persistence.
//!
//! Prerequisites:
//! - Running Bitcoin regtest (./scripts/start-regtest.sh)
//! - Running F1r3node with FIREFLY_* environment variables set

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{check_f1r3node_available, setup_wallet_with_genesis_utxo};

/// Test issuing a single RGB asset
///
/// Verifies:
/// - Asset can be issued via WalletManager
/// - AssetInfo returned with correct metadata
/// - State file created with contract metadata
#[tokio::test]
async fn test_issue_single_asset() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        println!("   Start F1r3node and set FIREFLY_* environment variables");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("issue_single_asset");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet with genesis UTXO
    let (mut manager, genesis_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Issue asset
    let ticker = "USD";
    let name = "US Dollar";
    let supply = 100_000_000u64;
    let precision = 2u8;

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

    // Verify AssetInfo has correct metadata
    assert_eq!(asset_info.ticker, ticker, "Ticker should match");
    assert_eq!(asset_info.name, name, "Name should match");
    assert_eq!(asset_info.supply, supply, "Supply should match");
    assert_eq!(asset_info.precision, precision, "Precision should match");
    assert!(
        !asset_info.contract_id.is_empty(),
        "Contract ID should not be empty"
    );
    assert!(
        !asset_info.genesis_seal.is_empty(),
        "Genesis seal should not be empty"
    );
    assert!(
        !asset_info.registry_uri.is_empty(),
        "Registry URI should not be empty"
    );

    // Verify state file was created
    let wallet_dir = env.wallet_dir(wallet_name);
    let state_file = wallet_dir.join("f1r3fly_state.json");
    assert!(
        state_file.exists(),
        "State file should exist at: {}",
        state_file.display()
    );

    // Verify state file contains contract metadata
    let state_content = std::fs::read_to_string(&state_file).expect("Failed to read state file");

    // Parse as generic JSON first for basic checks
    let state_json: serde_json::Value =
        serde_json::from_str(&state_content).expect("Failed to parse state file as JSON");

    assert!(
        state_json["contracts_metadata"].is_object(),
        "State should have contracts_metadata"
    );
    assert!(
        state_json["genesis_utxos"].is_object(),
        "State should have genesis_utxos"
    );

    // Parse as F1r3flyState for detailed validation
    let state: f1r3fly_rgb_wallet::f1r3fly::F1r3flyState =
        serde_json::from_str(&state_content).expect("Failed to deserialize F1r3flyState");

    // Verify derivation_index field
    assert!(
        state.derivation_index > 0,
        "Derivation index should be non-zero, got: {}",
        state.derivation_index
    );

    // Verify correct number of contracts and genesis UTXOs
    assert_eq!(
        state.contracts_metadata.len(),
        1,
        "Should have exactly 1 contract in state"
    );
    assert_eq!(
        state.genesis_utxos.len(),
        1,
        "Should have exactly 1 genesis UTXO in state"
    );

    // Verify tracker_state field exists
    assert!(state.tracker_state.is_some(), "Should have tracker state");

    // Verify contract metadata structure
    let (contract_id, metadata) = state
        .contracts_metadata
        .iter()
        .next()
        .expect("Should have at least one contract");

    assert_eq!(
        contract_id, &asset_info.contract_id,
        "Contract ID in state should match issued asset"
    );
    assert!(
        !metadata.registry_uri.is_empty(),
        "Contract metadata should have non-empty registry URI"
    );
    assert!(
        !metadata.methods.is_empty(),
        "Contract metadata should have methods"
    );
    assert!(
        !metadata.rholang_source.is_empty(),
        "Contract metadata should have non-empty Rholang source"
    );

    // Verify genesis UTXO structure
    let (genesis_contract_id, genesis_info) = state
        .genesis_utxos
        .iter()
        .next()
        .expect("Should have at least one genesis UTXO");

    assert_eq!(
        genesis_contract_id, &asset_info.contract_id,
        "Genesis UTXO contract ID should match issued asset"
    );
    assert_eq!(
        genesis_info.contract_id, asset_info.contract_id,
        "Genesis info contract_id field should match"
    );
    assert_eq!(
        genesis_info.ticker, ticker,
        "Genesis info ticker should match"
    );
    assert_eq!(genesis_info.name, name, "Genesis info name should match");
    assert_eq!(
        genesis_info.supply, supply,
        "Genesis info supply should match"
    );
    assert_eq!(
        genesis_info.precision, precision,
        "Genesis info precision should match"
    );
    assert!(
        !genesis_info.txid.is_empty(),
        "Genesis info should have non-empty txid"
    );
    assert!(
        genesis_info.vout < 100,
        "Genesis info vout should be reasonable (< 100)"
    );
}

/// Test issuing multiple RGB assets in the same wallet
///
/// Verifies:
/// - Multiple assets can be issued
/// - Each asset has unique contract ID
/// - list_assets() returns all assets
/// - State file tracks all assets
#[tokio::test]
async fn test_issue_multiple_assets() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("issue_multiple_assets");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet with initial genesis UTXO
    let (mut manager, genesis_utxo_1) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Create second genesis UTXO
    let amount_sats = 1_000_000u64; // 0.01 BTC
    let fee_config = f1r3fly_rgb_wallet::bitcoin::utxo::FeeRateConfig::medium_priority();
    let genesis_result_2 = manager
        .create_utxo(amount_sats, &fee_config, false)
        .expect("Failed to create second UTXO");
    let genesis_txid_2 = genesis_result_2.txid.clone();
    env.wait_for_confirmation(&genesis_txid_2, 1)
        .await
        .expect("Failed to confirm second UTXO");

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

    // Format the second genesis UTXO using the outpoint from the result
    let genesis_utxo_2 = format!(
        "{}:{}",
        genesis_result_2.txid, genesis_result_2.outpoint.vout
    );

    // Issue first asset (USD)
    let request_1 = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "USD".to_string(),
        name: "US Dollar".to_string(),
        supply: 100_000_000,
        precision: 2,
        genesis_utxo: genesis_utxo_1,
    };

    let asset_1 = manager
        .issue_asset(request_1)
        .await
        .expect("Failed to issue first asset");

    // Issue second asset (EUR)
    let request_2 = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "EUR".to_string(),
        name: "Euro".to_string(),
        supply: 200_000_000,
        precision: 2,
        genesis_utxo: genesis_utxo_2,
    };

    let asset_2 = manager
        .issue_asset(request_2)
        .await
        .expect("Failed to issue second asset");

    // Verify assets have unique contract IDs
    assert_ne!(
        asset_1.contract_id, asset_2.contract_id,
        "Assets should have different contract IDs"
    );

    // List all assets
    let assets = manager.list_assets().expect("Failed to list assets");

    // Verify both assets are returned
    assert_eq!(assets.len(), 2, "Should have exactly 2 assets");

    let asset_ids: Vec<String> = assets.iter().map(|a| a.contract_id.clone()).collect();
    assert!(
        asset_ids.contains(&asset_1.contract_id),
        "List should contain first asset"
    );
    assert!(
        asset_ids.contains(&asset_2.contract_id),
        "List should contain second asset"
    );

    // Verify state file tracks both assets
    let wallet_dir = env.wallet_dir(wallet_name);
    let state_file = wallet_dir.join("f1r3fly_state.json");
    let state_content = std::fs::read_to_string(&state_file).expect("Failed to read state file");
    let state: serde_json::Value =
        serde_json::from_str(&state_content).expect("Failed to parse state file");

    let contracts_metadata = state["contracts_metadata"]
        .as_object()
        .expect("contracts_metadata should be an object");

    assert_eq!(
        contracts_metadata.len(),
        2,
        "State should track 2 contracts"
    );
}

/// Test listing assets when wallet has no assets
///
/// Verifies:
/// - list_assets() returns empty vector for new wallet
#[test]
fn test_list_assets_empty() {
    // Setup test environment
    let env = TestBitcoinEnv::new("list_assets_empty");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet without issuing any assets
    let mut manager = f1r3fly_rgb_wallet::manager::WalletManager::new(env.config().clone())
        .expect("Failed to create manager");

    manager
        .create_wallet(wallet_name, password)
        .expect("Failed to create wallet");

    manager
        .load_wallet(wallet_name, password)
        .expect("Failed to load wallet");

    // List assets
    let assets = manager.list_assets().expect("Failed to list assets");

    // Verify empty list
    assert_eq!(assets.len(), 0, "New wallet should have no assets");
}

/// Test retrieving specific asset information
///
/// Verifies:
/// - get_asset_info() returns correct metadata
/// - Metadata matches issued parameters
/// - Genesis seal information is correct
#[tokio::test]
async fn test_get_asset_info() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("get_asset_info");
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

    let contract_id = asset_info.contract_id.clone();

    // Retrieve asset info
    let retrieved_info = manager
        .get_asset_info(&contract_id)
        .expect("Failed to get asset info");

    // Verify all metadata matches
    assert_eq!(
        retrieved_info.contract_id, contract_id,
        "Contract ID should match"
    );
    assert_eq!(retrieved_info.ticker, ticker, "Ticker should match");
    assert_eq!(retrieved_info.name, name, "Name should match");
    assert_eq!(retrieved_info.supply, supply, "Supply should match");
    assert_eq!(
        retrieved_info.precision, precision,
        "Precision should match"
    );
    assert_eq!(
        retrieved_info.genesis_seal, asset_info.genesis_seal,
        "Genesis seal should match"
    );
    assert_eq!(
        retrieved_info.registry_uri, asset_info.registry_uri,
        "Registry URI should match"
    );
}

/// Test issuing asset with invalid UTXO format
///
/// Verifies:
/// - Invalid UTXO strings are rejected with appropriate error
#[tokio::test]
async fn test_issue_asset_invalid_utxo_format() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("invalid_utxo_format");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet (no need for genesis UTXO since we're testing invalid format)
    let mut manager = f1r3fly_rgb_wallet::manager::WalletManager::new(env.config().clone())
        .expect("Failed to create manager");

    manager
        .create_wallet(wallet_name, password)
        .expect("Failed to create wallet");

    manager
        .load_wallet(wallet_name, password)
        .expect("Failed to load wallet");

    // Test various invalid UTXO formats
    let invalid_formats = vec![
        "invalid",
        "abc",
        "123:xyz",
        ":",
        "txid:",
        ":0",
        "not_a_txid:not_a_vout",
    ];

    for invalid_utxo in invalid_formats {
        let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
            ticker: "TEST".to_string(),
            name: "Test Token".to_string(),
            supply: 1000,
            precision: 0,
            genesis_utxo: invalid_utxo.to_string(),
        };

        let result = manager.issue_asset(request).await;

        // Verify it returns an error
        assert!(
            result.is_err(),
            "Invalid UTXO format '{}' should return error",
            invalid_utxo
        );

        let error_msg = result.unwrap_err().to_string();

        // Verify error message is meaningful
        assert!(
            error_msg.contains("txid")
                || error_msg.contains("vout")
                || error_msg.contains("parse")
                || error_msg.contains("invalid")
                || error_msg.contains("format"),
            "Error message should indicate UTXO format issue: {}",
            error_msg
        );
    }
}

/// Test issuing asset with UTXO not owned by wallet
///
/// Verifies:
/// - UTXOs not in wallet are rejected with appropriate error
#[tokio::test]
async fn test_issue_asset_utxo_not_owned() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("utxo_not_owned");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet with some funds (but we won't use the real UTXOs)
    let (mut manager, _real_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    // Create a fake UTXO that doesn't exist in the wallet
    let fake_txid = "0000000000000000000000000000000000000000000000000000000000000000";
    let fake_utxo = format!("{}:0", fake_txid);

    let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "FAKE".to_string(),
        name: "Fake Token".to_string(),
        supply: 1000,
        precision: 0,
        genesis_utxo: fake_utxo.clone(),
    };

    let result = manager.issue_asset(request).await;

    // Verify it returns an error
    assert!(result.is_err(), "UTXO not in wallet should return error");

    let error_msg = result.unwrap_err().to_string();

    // Verify error message indicates UTXO not found
    assert!(
        error_msg.contains("not found")
            || error_msg.contains("UTXO")
            || error_msg.contains("does not exist"),
        "Error message should indicate UTXO not found: {}",
        error_msg
    );
}

/// Test retrieving asset info for non-existent contract
///
/// Verifies:
/// - get_asset_info() returns error for non-existent contract
#[tokio::test]
async fn test_get_asset_info_not_found() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        println!("⚠️  Skipping test: F1r3node not available");
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("asset_info_not_found");
    let wallet_name = env.unique_wallet_name();
    let password = "test_password";

    // Create wallet and issue one asset
    let (mut manager, genesis_utxo) = setup_wallet_with_genesis_utxo(&env, wallet_name, password)
        .await
        .expect("Failed to setup wallet");

    let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
        ticker: "REAL".to_string(),
        name: "Real Token".to_string(),
        supply: 1000,
        precision: 0,
        genesis_utxo,
    };

    let _asset_info = manager
        .issue_asset(request)
        .await
        .expect("Failed to issue asset");

    // Try to get info for a fake contract ID
    let fake_contract_id = "fake_contract_id_that_does_not_exist";

    let result = manager.get_asset_info(fake_contract_id);

    // Verify it returns an error
    assert!(result.is_err(), "Non-existent contract should return error");

    let error_msg = result.unwrap_err().to_string();

    // Verify error message indicates contract not found
    assert!(
        error_msg.contains("not found")
            || error_msg.contains("contract")
            || error_msg.contains("does not exist"),
        "Error message should indicate contract not found: {}",
        error_msg
    );
}
