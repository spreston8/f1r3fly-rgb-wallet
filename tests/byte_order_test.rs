//! Byte Order Validation Tests
//!
//! These tests ensure that all Bitcoin TXID representations are consistent
//! across the codebase, using big-endian (display) format when sending to F1r3node.

use bitcoin::Txid as BtcTxid;
use std::str::FromStr;

#[test]
fn test_serialize_seal_produces_big_endian() {
    use bp::seals::{Noise, TxoSealExt};
    use bp::Outpoint;
    use f1r3fly_rgb::contract::F1r3flyRgbContract;
    use f1r3fly_rgb::{Txid as BpTxid, TxoSeal};
    use strict_types::StrictDumb;

    // Create a test TXID in big-endian display format (as users see it)
    let display_txid = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

    // Parse it using bitcoin crate (stores internally as little-endian)
    let btc_txid = BtcTxid::from_str(display_txid).unwrap();

    // Get the bytes from bitcoin::Txid (these are in little-endian internally)
    let btc_bytes: [u8; 32] = *btc_txid.as_ref();

    // Create bp::Txid from the same bytes (also stores as little-endian internally)
    let bp_txid = BpTxid::from(btc_bytes);

    // Create a TxoSeal
    let outpoint = Outpoint::new(bp_txid, 0);
    let seal = TxoSeal {
        primary: outpoint,
        secondary: TxoSealExt::Noise(Noise::strict_dumb()),
    };

    // Serialize the seal
    let serialized = F1r3flyRgbContract::serialize_seal(&seal);

    // The serialized format should be in big-endian (display format)
    let expected = format!("{}:0", display_txid);

    assert_eq!(
        serialized, expected,
        "serialize_seal should produce big-endian format matching the original display txid"
    );

    println!("✓ serialize_seal produces big-endian: {}", serialized);
}

#[test]
fn test_asset_genesis_utxo_normalization() {
    // Simulate what happens in asset.rs when normalizing genesis UTXO
    let display_txid = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let vout = "1";

    // The normalized genesis seal should preserve big-endian format
    let normalized = format!("{}:{}", display_txid, vout);

    // This should match what we send to F1r3node
    assert_eq!(
        normalized,
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890:1"
    );

    println!(
        "✓ Genesis UTXO normalization preserves big-endian: {}",
        normalized
    );
}

#[test]
fn test_claim_real_utxo_format() {
    use std::str::FromStr;

    // Simulate what happens in consignment.rs when formatting real_utxo for claim
    let display_txid_str = "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";

    // Parse it like we do in the claim code
    let real_txid = BtcTxid::from_str(display_txid_str).unwrap();

    // Format using to_string() (should preserve big-endian display format)
    let real_utxo = format!("{}:{}", real_txid.to_string(), 0);

    // Should match the original display format
    let expected = format!("{}:0", display_txid_str);

    assert_eq!(
        real_utxo, expected,
        "real_utxo should use big-endian display format"
    );

    println!("✓ Claim real_utxo uses big-endian: {}", real_utxo);
}

#[test]
fn test_bitcoin_txid_display_format() {
    // Test that bitcoin::Txid::to_string() gives big-endian format
    let display_txid = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let txid = BtcTxid::from_str(display_txid).unwrap();
    let txid_string = txid.to_string();

    assert_eq!(
        txid_string, display_txid,
        "bitcoin::Txid::to_string() should return big-endian display format"
    );

    println!(
        "✓ bitcoin::Txid::to_string() produces big-endian: {}",
        txid_string
    );
}

#[test]
fn test_bp_txid_internal_storage_vs_display() {
    use amplify::ByteArray;
    use f1r3fly_rgb::Txid as BpTxid;

    // Original display format (big-endian)
    let display_txid = "1111111111111111222222222222222233333333333333334444444444444444";

    // Parse via bitcoin crate
    let btc_txid = BtcTxid::from_str(display_txid).unwrap();
    let btc_bytes: [u8; 32] = *btc_txid.as_ref();

    // Create bp::Txid from same bytes
    let bp_txid = BpTxid::from(btc_bytes);

    // Get bytes back from bp::Txid
    let bp_bytes = bp_txid.to_byte_array();

    // Bytes should match (both store in little-endian internally)
    assert_eq!(
        btc_bytes, bp_bytes,
        "bitcoin::Txid and bp::Txid should store bytes identically"
    );

    // Now test what serialize_seal does: reverse bytes to get big-endian
    let mut reversed_bytes = bp_bytes;
    reversed_bytes.reverse();
    let serialized_txid = hex::encode(reversed_bytes);

    assert_eq!(
        serialized_txid, display_txid,
        "Reversing bp::Txid bytes should give original big-endian display format"
    );

    println!(
        "✓ bp::Txid byte reversal produces correct big-endian: {}",
        serialized_txid
    );
}

#[test]
fn test_end_to_end_consistency() {
    use bp::seals::{Noise, TxoSealExt};
    use bp::Outpoint;
    use f1r3fly_rgb::contract::F1r3flyRgbContract;
    use f1r3fly_rgb::Txid as BpTxid;
    use f1r3fly_rgb::TxoSeal;
    use strict_types::StrictDumb;

    // Start with a display TXID (as user would see in explorer)
    let original_display = "aaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbccccccccccccccccdddddddddddddddd";

    // Step 1: Asset issuance - normalize genesis UTXO (from asset.rs)
    let genesis_utxo = format!("{}:0", original_display);

    // Step 2: Create bp::Txid for RGB operations (from balance.rs convert_outpoint_to_seal)
    let btc_txid = BtcTxid::from_str(original_display).unwrap();
    let btc_bytes: [u8; 32] = *btc_txid.as_ref();
    let bp_txid = BpTxid::from(btc_bytes);

    // Step 3: Create seal and serialize (for balance query)
    let outpoint = Outpoint::new(bp_txid, 0);
    let seal = TxoSeal {
        primary: outpoint,
        secondary: TxoSealExt::Noise(Noise::strict_dumb()),
    };
    let balance_query_utxo = F1r3flyRgbContract::serialize_seal(&seal);

    // Step 4: Format for claim (from consignment.rs)
    let claim_utxo = format!("{}:0", btc_txid.to_string());

    // ALL FOUR SHOULD MATCH!
    println!("\n=== End-to-End Consistency Check ===");
    println!("Original display:     {}", original_display);
    println!("Genesis UTXO:         {}", genesis_utxo);
    println!("Balance query UTXO:   {}", balance_query_utxo);
    println!("Claim UTXO:           {}", claim_utxo);

    assert_eq!(
        genesis_utxo, balance_query_utxo,
        "Genesis UTXO and balance query UTXO should match"
    );

    assert_eq!(
        balance_query_utxo, claim_utxo,
        "Balance query UTXO and claim UTXO should match"
    );

    assert_eq!(
        genesis_utxo, claim_utxo,
        "Genesis UTXO and claim UTXO should match"
    );

    println!("✅ ALL FORMATS CONSISTENT!");
}
