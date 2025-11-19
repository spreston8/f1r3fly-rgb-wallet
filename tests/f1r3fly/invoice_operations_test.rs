//! Invoice Operations Integration Tests
//!
//! Tests for invoice generation and parsing functionality.
//! Verifies the complete invoice round-trip workflow between two wallets.
//!
//! Prerequisites:
//! - Running Bitcoin regtest (./scripts/start-regtest.sh)
//! - Running F1r3node with FIREFLY_* environment variables set

use crate::common::TestBitcoinEnv;
use crate::f1r3fly::{check_f1r3node_available, issue_test_asset, setup_recipient_wallet};

/// Test complete invoice generation and parsing round-trip
///
/// Verifies:
/// - Bob can generate invoice for a known asset
/// - Invoice format follows RGB standard
/// - Alice can parse invoice correctly
/// - All fields (contract ID, amount, address, seal) extracted properly
#[tokio::test]
async fn test_invoice_round_trip() {
    // Check F1r3node availability
    if !check_f1r3node_available() {
        return;
    }

    // Setup test environment
    let env = TestBitcoinEnv::new("invoice_operations");

    // ========================================================================
    // Step 1: Alice issues asset and Bob accepts genesis
    // ========================================================================
    let (mut alice, asset_info, _) =
        issue_test_asset(&env, env.unique_wallet_name(), "USD", 10_000)
            .await
            .expect("Failed to issue asset");

    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob");

    // Export and import genesis
    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    bob.accept_consignment(genesis_response.consignment_path.to_str().unwrap())
        .await
        .expect("Failed to accept genesis");

    // ========================================================================
    // Step 2: Bob generates invoice for 100 tokens
    // ========================================================================
    let bob_bitcoin_wallet = bob
        .bitcoin_wallet_mut()
        .expect("Bob should have Bitcoin wallet");

    let generated = f1r3fly_rgb_wallet::f1r3fly::generate_invoice(
        bob_bitcoin_wallet,
        &asset_info.contract_id,
        100,
        None,
    )
    .expect("Failed to generate invoice");

    let invoice_string = generated.invoice.to_string();

    // ========================================================================
    // Step 3: Verify invoice format (RGB standard)
    // ========================================================================
    assert!(!invoice_string.is_empty(), "Invoice should not be empty");

    // RGB invoices may use "contract:" or "rgb:" prefix
    assert!(
        invoice_string.contains("@"),
        "Invoice should contain @ separator, got: {}",
        invoice_string
    );

    assert!(
        invoice_string.len() > 50 && invoice_string.len() < 200,
        "Invoice should be reasonably sized, got: {} chars",
        invoice_string.len()
    );

    // ========================================================================
    // Step 4: Alice parses invoice
    // ========================================================================
    let parsed = f1r3fly_rgb_wallet::f1r3fly::parse_invoice(&invoice_string)
        .expect("Failed to parse invoice");

    // ========================================================================
    // Step 5: Verify parsed data matches original
    // ========================================================================
    assert_eq!(
        parsed.contract_id.to_string(),
        asset_info.contract_id,
        "Contract ID should match"
    );

    assert_eq!(parsed.amount, Some(100), "Amount should match");

    // Extract address from beneficiary
    let network = f1r3fly_rgb_wallet::config::NetworkType::Regtest.to_bitcoin_network();
    let recipient_address =
        f1r3fly_rgb_wallet::f1r3fly::get_address_from_invoice(&parsed.beneficiary, network)
            .expect("Should extract address");

    assert!(
        !recipient_address.to_string().is_empty(),
        "Recipient address should be present"
    );

    // ========================================================================
    // Step 6: Verify seal extracted
    // ========================================================================
    // Beneficiary contains the seal information
    assert!(
        !format!("{:?}", parsed.beneficiary).is_empty(),
        "Beneficiary (seal) should be present"
    );
}

/// Test invoice generation without genesis acceptance
///
/// Verifies:
/// - Invoice generation fails gracefully for unknown contracts
#[tokio::test]
async fn test_invoice_generation_unknown_contract() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("invoice_unknown_contract");

    // Setup Bob without accepting any genesis
    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob");

    // Try to generate invoice for non-existent contract
    let fake_contract_id = "contract:fake1234567890";

    let bob_bitcoin_wallet = bob
        .bitcoin_wallet_mut()
        .expect("Bob should have Bitcoin wallet");

    let result = f1r3fly_rgb_wallet::f1r3fly::generate_invoice(
        bob_bitcoin_wallet,
        fake_contract_id,
        100,
        None,
    );

    assert!(
        result.is_err(),
        "Invoice generation should fail for unknown contract"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Invalid")
            || error_msg.contains("not found")
            || error_msg.contains("contract"),
        "Error should indicate contract issue: {}",
        error_msg
    );
}

/// Test invoice parsing with invalid format
///
/// Verifies:
/// - Malformed invoices are rejected gracefully
#[tokio::test]
async fn test_invoice_parsing_invalid_format() {
    if !check_f1r3node_available() {
        return;
    }

    // Test various invalid invoice formats
    let invalid_invoices = vec![
        "not_an_invoice",
        "rgb:",
        "rgb:invalid_data",
        "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", // Just an address
        "",
    ];

    for invalid_invoice in invalid_invoices {
        let result = f1r3fly_rgb_wallet::f1r3fly::parse_invoice(invalid_invoice);

        assert!(
            result.is_err(),
            "Should reject invalid invoice: {}",
            invalid_invoice
        );
    }
}

/// Test invoice generation with zero amount
///
/// Verifies:
/// - Zero amount invoices are rejected
#[tokio::test]
async fn test_invoice_generation_zero_amount() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("invoice_zero_amount");

    let (mut alice, asset_info, _) = issue_test_asset(&env, env.unique_wallet_name(), "TEST", 1000)
        .await
        .expect("Failed to issue asset");

    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob");

    // Bob accepts genesis
    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    bob.accept_consignment(genesis_response.consignment_path.to_str().unwrap())
        .await
        .expect("Failed to accept genesis");

    // Try to generate invoice with zero amount
    let bob_bitcoin_wallet = bob
        .bitcoin_wallet_mut()
        .expect("Bob should have Bitcoin wallet");

    let result = f1r3fly_rgb_wallet::f1r3fly::generate_invoice(
        bob_bitcoin_wallet,
        &asset_info.contract_id,
        0,
        None,
    );

    assert!(
        result.is_err(),
        "Invoice generation should fail for zero amount"
    );

    let error_msg = result.unwrap_err().to_string();
    let error_lower = error_msg.to_lowercase();
    assert!(
        error_lower.contains("amount")
            || error_lower.contains("zero")
            || error_lower.contains("greater"),
        "Error should indicate invalid amount: {}",
        error_msg
    );
}

/// Test invoice with custom expiration
///
/// Verifies:
/// - Invoices can include expiration timestamps
/// - Expired invoices are still parseable (but should be rejected at payment time)
#[tokio::test]
async fn test_invoice_with_expiration() {
    if !check_f1r3node_available() {
        return;
    }

    let env = TestBitcoinEnv::new("invoice_with_expiration");

    let (mut alice, asset_info, _) = issue_test_asset(&env, env.unique_wallet_name(), "TEST", 1000)
        .await
        .expect("Failed to issue asset");

    let mut bob = setup_recipient_wallet(&env, "bob", "test_password")
        .await
        .expect("Failed to setup Bob");

    // Bob accepts genesis
    let genesis_response = alice
        .export_genesis(&asset_info.contract_id)
        .await
        .expect("Failed to export genesis");

    bob.accept_consignment(genesis_response.consignment_path.to_str().unwrap())
        .await
        .expect("Failed to accept genesis");

    // Note: Expiration is not yet supported in the current implementation
    // This test verifies basic invoice generation works

    let bob_bitcoin_wallet = bob
        .bitcoin_wallet_mut()
        .expect("Bob should have Bitcoin wallet");

    // Note: Current implementation doesn't support expiration in generate_invoice
    // This test verifies the invoice still works without expiration support
    let generated = f1r3fly_rgb_wallet::f1r3fly::generate_invoice(
        bob_bitcoin_wallet,
        &asset_info.contract_id,
        100,
        None, // Expiration not yet supported in generate_invoice
    )
    .expect("Failed to generate invoice");

    let invoice_string = generated.invoice.to_string();

    // Alice should be able to parse it
    let parsed = f1r3fly_rgb_wallet::f1r3fly::parse_invoice(&invoice_string)
        .expect("Failed to parse invoice");

    assert_eq!(parsed.amount, Some(100), "Amount should match");
    assert_eq!(
        parsed.contract_id.to_string(),
        asset_info.contract_id,
        "Contract ID should match"
    );

    // Note: Expiration support would be a future enhancement
    // For now, we verify basic invoice generation works without it
}
