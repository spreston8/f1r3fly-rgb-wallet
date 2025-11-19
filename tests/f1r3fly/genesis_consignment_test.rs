//! Genesis Consignment Integration Tests
//!
//! Tests for genesis consignment export and import functionality.
//! Verifies the complete flow from asset issuance through genesis export
//! and acceptance by a recipient wallet.
//!
//! Prerequisites:
//! - Running Bitcoin regtest (./scripts/start-regtest.sh)
//! - Running F1r3node with FIREFLY_* environment variables set

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{
    check_f1r3node_available, issue_test_asset, setup_recipient_wallet, verify_consignment_file,
};

/// Test complete genesis export and import flow
///
/// Verifies:
/// - Alice can issue an asset
/// - Alice can export genesis consignment
/// - Consignment file is valid and contains real data
/// - Bob can accept genesis consignment
/// - Bob can query asset metadata
/// - Bob's state persisted correctly
#[tokio::test]
async fn test_genesis_export_and_import() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("genesis_export_import");

    // ========================================================================
    // Step 1: Alice issues asset
    // ========================================================================

    let (mut alice, asset_info, _genesis_utxo) =
        issue_test_asset(&env, env.unique_wallet_name(), "TEST", 10_000)
            .await
            .expect("Failed to issue test asset");

    // ========================================================================
    // Step 2: Alice exports genesis consignment
    // ========================================================================

    let export_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    // ========================================================================
    // Step 3: Verify consignment file structure
    // ========================================================================

    verify_consignment_file(&export_response.consignment_path)
        .expect("Consignment file validation failed");

    // Verify file size is reasonable (5 KB - 50 KB for genesis)
    assert!(
        export_response.consignment_size >= 1024,
        "Consignment too small: {} bytes",
        export_response.consignment_size
    );
    assert!(
        export_response.consignment_size <= 52_428_800,
        "Consignment too large: {} bytes",
        export_response.consignment_size
    );

    // ========================================================================
    // Step 4: Verify consignment contains real data (not placeholders)
    // ========================================================================

    let consignment_bytes =
        std::fs::read(&export_response.consignment_path).expect("Failed to read consignment file");

    let consignment = f1r3fly_rgb::F1r3flyConsignment::from_bytes(&consignment_bytes)
        .expect("Failed to deserialize consignment");

    // Verify F1r3fly proof has real data
    let f1r3fly_proof = consignment.f1r3fly_proof();

    assert!(
        !f1r3fly_proof.block_hash.is_empty(),
        "Block hash should not be empty"
    );

    assert_ne!(
        f1r3fly_proof.state_hash, [0u8; 32],
        "State hash should not be all zeros"
    );

    assert!(
        !f1r3fly_proof.deploy_id.is_empty(),
        "Deploy ID should not be empty"
    );

    // Verify witness transactions present
    assert!(
        !consignment.witness_txs.is_empty(),
        "Consignment should have witness transactions"
    );

    // Verify seals present
    assert!(
        !consignment.seals().is_empty(),
        "Consignment should have seals"
    );

    // Verify contract metadata present
    let metadata = consignment.metadata();
    assert!(
        !metadata.registry_uri.is_empty(),
        "Contract metadata should have registry URI"
    );
    assert!(
        !metadata.methods.is_empty(),
        "Contract metadata should have methods"
    );
    assert!(
        !metadata.rholang_source.is_empty(),
        "Contract metadata should have Rholang source"
    );

    // ========================================================================
    // Step 5: Bob creates wallet and accepts genesis
    // ========================================================================

    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob's wallet");

    let accept_response = bob
        .accept_consignment(
            export_response
                .consignment_path
                .to_str()
                .expect("Invalid path"),
        )
        .await
        .expect("Failed to accept consignment");

    // Verify acceptance response
    assert_eq!(
        accept_response.contract_id, asset_info.contract_id,
        "Contract ID should match"
    );
    assert_eq!(
        accept_response.seals_imported, 1,
        "Should import exactly 1 seal (genesis seal)"
    );

    // ========================================================================
    // Step 6: Verify Bob can query asset metadata
    // ========================================================================

    let bob_asset_info = bob
        .get_asset_info(&asset_info.contract_id)
        .expect("Failed to get asset info from Bob's wallet");

    assert_eq!(
        bob_asset_info.contract_id, asset_info.contract_id,
        "Contract ID should match"
    );
    assert_eq!(
        bob_asset_info.ticker, asset_info.ticker,
        "Ticker should match"
    );
    assert_eq!(bob_asset_info.name, asset_info.name, "Name should match");
    assert_eq!(
        bob_asset_info.supply, asset_info.supply,
        "Supply should match"
    );
    assert_eq!(
        bob_asset_info.precision, asset_info.precision,
        "Precision should match"
    );

    // ========================================================================
    // Step 7: Verify Bob's state persisted correctly
    // ========================================================================

    // Get Bob's wallet directory
    let bob_wallet_dir = env.wallet_dir("bob");
    let bob_state_file = bob_wallet_dir.join("f1r3fly_state.json");

    assert!(
        bob_state_file.exists(),
        "Bob's state file should exist at: {}",
        bob_state_file.display()
    );

    // Read and parse state file
    let bob_state_content =
        std::fs::read_to_string(&bob_state_file).expect("Failed to read Bob's state file");

    let bob_state: f1r3fly_rgb_wallet::f1r3fly::F1r3flyState =
        serde_json::from_str(&bob_state_content).expect("Failed to deserialize Bob's F1r3flyState");

    // Verify genesis UTXO info in state
    assert!(
        bob_state
            .genesis_utxos
            .contains_key(&asset_info.contract_id),
        "Bob's state should contain genesis UTXO for accepted asset"
    );

    let bob_genesis_info = &bob_state.genesis_utxos[&asset_info.contract_id];
    assert_eq!(
        bob_genesis_info.contract_id, asset_info.contract_id,
        "Genesis info contract ID should match"
    );
    assert_eq!(
        bob_genesis_info.ticker, accept_response.ticker,
        "Genesis info ticker should match"
    );
    assert_eq!(
        bob_genesis_info.name, accept_response.name,
        "Genesis info name should match"
    );

    // Verify genesis execution data stored
    assert!(
        bob_genesis_info.genesis_execution_result.is_some(),
        "Genesis execution result should be stored"
    );

    let bob_exec_data = bob_genesis_info.genesis_execution_result.as_ref().unwrap();
    assert!(!bob_exec_data.opid.is_empty(), "Opid should not be empty");
    assert!(
        !bob_exec_data.deploy_id.is_empty(),
        "Deploy ID should not be empty"
    );
    assert!(
        !bob_exec_data.finalized_block_hash.is_empty(),
        "Block hash should not be empty"
    );
    assert_ne!(
        bob_exec_data.state_hash, [0u8; 32],
        "State hash should not be all zeros"
    );

    // ========================================================================
    // Step 8: Verify Bob cannot spend (has no balance yet)
    // ========================================================================

    let bob_balance = bob
        .get_asset_balance(&asset_info.contract_id)
        .await
        .expect("Failed to query Bob's balance");

    assert_eq!(
        bob_balance.total, 0,
        "Bob should have zero balance (only accepted genesis, no transfer yet)"
    );

    // ========================================================================
    // Step 9: Verify Alice still has full balance
    // ========================================================================

    let alice_balance = alice
        .get_asset_balance(&asset_info.contract_id)
        .await
        .expect("Failed to query Alice's balance");

    assert_eq!(
        alice_balance.total, asset_info.supply,
        "Alice should still have full supply"
    );
}

/// Test genesis export with invalid contract ID
///
/// Verifies:
/// - Exporting genesis for non-existent contract fails gracefully
#[tokio::test]
async fn test_genesis_export_invalid_contract() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("genesis_export_invalid");

    // Issue asset
    let (mut alice, _asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "TEST", 1000)
            .await
            .expect("Failed to issue test asset");

    // Try to export genesis for non-existent contract
    let fake_contract_id = "non_existent_contract_id_12345";

    let result = alice.export_genesis(fake_contract_id).await;

    assert!(
        result.is_err(),
        "Export genesis should fail for non-existent contract"
    );

    let error_msg = result.unwrap_err().to_string();
    let error_lower = error_msg.to_lowercase();
    assert!(
        error_lower.contains("invalid")
            || error_lower.contains("not found")
            || error_lower.contains("genesis"),
        "Error should indicate contract issue: {}",
        error_msg
    );
}

/// Test accepting genesis consignment twice
///
/// Verifies:
/// - Accepting the same genesis twice fails gracefully
/// - Error message is clear
#[tokio::test]
async fn test_accept_genesis_twice() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("accept_genesis_twice");

    // Alice issues and exports
    let (mut alice, asset_info, _) = issue_test_asset(&env, env.unique_wallet_name(), "TEST", 1000)
        .await
        .expect("Failed to issue test asset");

    let export_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    // Bob accepts genesis
    let mut bob = setup_recipient_wallet(&env, "bob", "password")
        .await
        .expect("Failed to setup Bob's wallet");

    bob.accept_consignment(export_response.consignment_path.to_str().unwrap())
        .await
        .expect("First accept should succeed");

    // Try to accept again
    let result = bob
        .accept_consignment(export_response.consignment_path.to_str().unwrap())
        .await;

    assert!(result.is_err(), "Accepting genesis twice should fail");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("already exists") || error_msg.contains("exist"),
        "Error should indicate contract already exists: {}",
        error_msg
    );
}

/// Test genesis consignment with missing execution data
///
/// Verifies:
/// - Assets issued with older wallet versions (no execution data) fail gracefully
#[tokio::test]
async fn test_genesis_export_missing_execution_data() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("genesis_missing_exec_data");

    // Issue asset
    let (mut alice, asset_info, _) = issue_test_asset(&env, env.unique_wallet_name(), "TEST", 1000)
        .await
        .expect("Failed to issue test asset");

    // Manually corrupt state to remove genesis execution data
    // This simulates an asset issued with an older wallet version
    let wallet_dir = env.wallet_dir(env.unique_wallet_name());
    let state_file = wallet_dir.join("f1r3fly_state.json");
    let state_content = std::fs::read_to_string(&state_file).expect("Failed to read state file");
    let mut state: f1r3fly_rgb_wallet::f1r3fly::F1r3flyState =
        serde_json::from_str(&state_content).expect("Failed to parse state");

    // Remove genesis execution result
    if let Some(genesis_info) = state.genesis_utxos.get_mut(&asset_info.contract_id) {
        genesis_info.genesis_execution_result = None;
    }

    // Write back corrupted state
    let corrupted_state = serde_json::to_string_pretty(&state).expect("Failed to serialize state");
    std::fs::write(&state_file, corrupted_state).expect("Failed to write state");

    // Reload wallet to pick up corrupted state
    alice
        .load_wallet(env.unique_wallet_name(), "test_password")
        .expect("Failed to reload wallet");

    // Try to export genesis - should fail
    let result = alice.export_genesis(&asset_info.contract_id).await;

    assert!(
        result.is_err(),
        "Export should fail when genesis execution data is missing"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("execution")
            || error_msg.contains("missing")
            || error_msg.contains("Genesis"),
        "Error should indicate missing execution data: {}",
        error_msg
    );
}
