//! Integration tests for key management
//!
//! Tests BIP39 mnemonic generation, BIP32 Bitcoin key derivation,
//! custom F1r3fly key derivation, and production-grade encryption.

use bip39::Mnemonic;
use f1r3fly_rgb_wallet::config::NetworkType;
use f1r3fly_rgb_wallet::storage::keys::{
    decrypt_mnemonic, decrypt_secret_key, derive_bitcoin_keys, derive_f1r3fly_key,
    encrypt_mnemonic, encrypt_secret_key, generate_mnemonic, KeyError,
};
use std::str::FromStr;

#[test]
fn test_mnemonic_generation_produces_valid_12_word_phrases() {
    // Generate multiple mnemonics to ensure randomness
    let mnemonic1 = generate_mnemonic().expect("Failed to generate mnemonic");
    let mnemonic2 = generate_mnemonic().expect("Failed to generate second mnemonic");

    // Verify both are valid 12-word mnemonics
    let mnemonic1_str = mnemonic1.to_string();
    let mnemonic2_str = mnemonic2.to_string();
    let words1: Vec<&str> = mnemonic1_str.split_whitespace().collect();
    let words2: Vec<&str> = mnemonic2_str.split_whitespace().collect();

    assert_eq!(words1.len(), 12, "Mnemonic should have exactly 12 words");
    assert_eq!(
        words2.len(),
        12,
        "Second mnemonic should have exactly 12 words"
    );

    // Verify mnemonics are different (randomness check)
    assert_ne!(
        mnemonic1.to_string(),
        mnemonic2.to_string(),
        "Generated mnemonics should be unique"
    );

    // Verify mnemonics can be parsed back (valid BIP39)
    let reparsed1 =
        Mnemonic::parse(&mnemonic1.to_string()).expect("Generated mnemonic should be valid BIP39");
    assert_eq!(
        mnemonic1.to_string(),
        reparsed1.to_string(),
        "Mnemonic should round-trip through parse"
    );

    // Verify entropy is correct length (128 bits for 12 words)
    let entropy1 = mnemonic1.to_entropy();
    assert_eq!(
        entropy1.len(),
        16,
        "12-word mnemonic should have 16 bytes (128 bits) of entropy"
    );
}

#[test]
fn test_bitcoin_key_derivation_produces_correct_bip84_keys_for_all_networks() {
    // Use a known test mnemonic
    let mnemonic = Mnemonic::parse(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    ).expect("Failed to parse test mnemonic");

    // Test all networks
    let networks = vec![
        NetworkType::Regtest,
        NetworkType::Signet,
        NetworkType::Testnet,
        NetworkType::Mainnet,
    ];

    for network in networks {
        let xprv = derive_bitcoin_keys(&mnemonic, network)
            .expect(&format!("Failed to derive keys for {:?}", network));

        // Verify it's an extended private key
        assert!(
            xprv.to_string().starts_with("xprv") || xprv.to_string().starts_with("tprv"),
            "Should produce valid extended private key for {:?}",
            network
        );

        // Verify we can derive child keys (BIP32 functionality)
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let derivation_path = bitcoin::bip32::DerivationPath::from_str("m/0/0")
            .expect("Failed to create derivation path");
        let child = xprv.derive_priv(&secp, &derivation_path);
        assert!(
            child.is_ok(),
            "Should be able to derive child keys for {:?}",
            network
        );

        // Verify depth is correct (should be at account level: m/84'/coin_type'/0')
        assert_eq!(
            xprv.depth, 3,
            "Account-level key should have depth 3 for {:?}",
            network
        );
    }

    // Verify mainnet and testnet produce different keys (different coin types)
    let mainnet_xprv = derive_bitcoin_keys(&mnemonic, NetworkType::Mainnet)
        .expect("Failed to derive mainnet keys");
    let testnet_xprv = derive_bitcoin_keys(&mnemonic, NetworkType::Testnet)
        .expect("Failed to derive testnet keys");

    assert_ne!(
        mainnet_xprv.to_string(),
        testnet_xprv.to_string(),
        "Mainnet and testnet keys should be different"
    );

    // Verify testnet, signet, and regtest use same coin type (should produce same keys)
    let signet_xprv =
        derive_bitcoin_keys(&mnemonic, NetworkType::Signet).expect("Failed to derive signet keys");
    let regtest_xprv = derive_bitcoin_keys(&mnemonic, NetworkType::Regtest)
        .expect("Failed to derive regtest keys");

    assert_eq!(
        testnet_xprv.private_key, signet_xprv.private_key,
        "Testnet and signet should use same key derivation"
    );
    assert_eq!(
        testnet_xprv.private_key, regtest_xprv.private_key,
        "Testnet and regtest should use same key derivation"
    );
}

#[test]
fn test_f1r3fly_key_derivation_produces_valid_secp256k1_keypair() {
    // Use a known test mnemonic
    let mnemonic = Mnemonic::parse(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    ).expect("Failed to parse test mnemonic");

    let (secret_key, public_key_hex) =
        derive_f1r3fly_key(&mnemonic).expect("Failed to derive F1r3fly key");

    // Verify secret key is valid (32 bytes)
    let secret_bytes = secret_key.secret_bytes();
    assert_eq!(
        secret_bytes.len(),
        32,
        "Secret key should be 32 bytes (256 bits)"
    );

    // Verify public key hex is valid format
    assert_eq!(
        public_key_hex.len(),
        66,
        "Compressed public key should be 66 hex characters (33 bytes)"
    );
    assert!(
        public_key_hex.starts_with("02") || public_key_hex.starts_with("03"),
        "Compressed public key should start with 02 or 03"
    );

    // Verify public key can be decoded
    let pubkey_bytes = hex::decode(&public_key_hex).expect("Public key should be valid hex");
    assert_eq!(
        pubkey_bytes.len(),
        33,
        "Compressed public key should be 33 bytes"
    );

    // Verify public key matches secret key
    let secp = secp256k1::Secp256k1::new();
    let derived_pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let derived_pubkey_hex = hex::encode(derived_pubkey.serialize());
    assert_eq!(
        public_key_hex, derived_pubkey_hex,
        "Public key should match secret key"
    );

    // Verify determinism: same mnemonic produces same keys
    let (secret_key2, public_key_hex2) =
        derive_f1r3fly_key(&mnemonic).expect("Failed to derive F1r3fly key second time");
    assert_eq!(
        secret_key.secret_bytes(),
        secret_key2.secret_bytes(),
        "Same mnemonic should produce same secret key"
    );
    assert_eq!(
        public_key_hex, public_key_hex2,
        "Same mnemonic should produce same public key"
    );

    // Verify different mnemonic produces different keys
    let different_mnemonic = Mnemonic::parse(
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    )
    .expect("Failed to parse different test mnemonic");
    let (different_secret, different_pubkey) = derive_f1r3fly_key(&different_mnemonic)
        .expect("Failed to derive keys from different mnemonic");

    assert_ne!(
        secret_key.secret_bytes(),
        different_secret.secret_bytes(),
        "Different mnemonic should produce different secret key"
    );
    assert_ne!(
        public_key_hex, different_pubkey,
        "Different mnemonic should produce different public key"
    );
}

#[test]
fn test_encryption_decryption_round_trip_with_production_pbkdf2() {
    let password = "strong_password_123!@#";

    // Test 1: Mnemonic encryption/decryption
    let original_mnemonic =
        generate_mnemonic().expect("Failed to generate mnemonic for encryption test");

    let encrypted_mnemonic =
        encrypt_mnemonic(&original_mnemonic, password).expect("Failed to encrypt mnemonic");

    // Verify encrypted data is hex-encoded and has minimum length
    assert!(
        encrypted_mnemonic.len() >= 88,
        "Encrypted mnemonic should have minimum length (salt 32 + nonce 24 + data + tag 32 hex chars)"
    );
    assert!(
        hex::decode(&encrypted_mnemonic).is_ok(),
        "Encrypted mnemonic should be valid hex"
    );

    let decrypted_mnemonic =
        decrypt_mnemonic(&encrypted_mnemonic, password).expect("Failed to decrypt mnemonic");

    assert_eq!(
        original_mnemonic.to_string(),
        decrypted_mnemonic.to_string(),
        "Decrypted mnemonic should match original"
    );

    // Test 2: Secret key encryption/decryption
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let (original_secret_key, _) =
        derive_f1r3fly_key(&mnemonic).expect("Failed to derive F1r3fly key");

    let encrypted_secret =
        encrypt_secret_key(&original_secret_key, password).expect("Failed to encrypt secret key");

    // Verify encrypted data is hex-encoded
    assert!(
        encrypted_secret.len() >= 88,
        "Encrypted secret key should have minimum length"
    );
    assert!(
        hex::decode(&encrypted_secret).is_ok(),
        "Encrypted secret key should be valid hex"
    );

    let decrypted_secret =
        decrypt_secret_key(&encrypted_secret, password).expect("Failed to decrypt secret key");

    assert_eq!(
        original_secret_key.secret_bytes(),
        decrypted_secret.secret_bytes(),
        "Decrypted secret key should match original"
    );

    // Test 3: Verify different encryptions of same data produce different ciphertext
    // (due to random salt and nonce)
    let encrypted_again =
        encrypt_mnemonic(&original_mnemonic, password).expect("Failed to encrypt mnemonic again");

    assert_ne!(
        encrypted_mnemonic, encrypted_again,
        "Same data encrypted twice should produce different ciphertext (random salt/nonce)"
    );

    // But both should decrypt to same value
    let decrypted_again =
        decrypt_mnemonic(&encrypted_again, password).expect("Failed to decrypt second encryption");

    assert_eq!(
        original_mnemonic.to_string(),
        decrypted_again.to_string(),
        "Both encryptions should decrypt to same value"
    );

    // Test 4: Verify PBKDF2 is being used (decryption should be relatively slow)
    // This is implicit - if we're using 600k iterations, decryption will take measurable time
    use std::time::Instant;
    let start = Instant::now();
    let _ = decrypt_mnemonic(&encrypted_mnemonic, password).expect("Decryption should succeed");
    let duration = start.elapsed();

    // PBKDF2 with 600k iterations should take at least a few milliseconds
    assert!(
        duration.as_millis() >= 1,
        "Decryption should take measurable time due to PBKDF2 iterations"
    );
}

#[test]
fn test_decryption_fails_with_wrong_password_and_invalid_data() {
    let correct_password = "correct_password";
    let wrong_password = "wrong_password";

    // Test 1: Wrong password for mnemonic decryption
    let mnemonic = generate_mnemonic().expect("Failed to generate mnemonic");
    let encrypted = encrypt_mnemonic(&mnemonic, correct_password).expect("Failed to encrypt");

    let result = decrypt_mnemonic(&encrypted, wrong_password);
    assert!(
        result.is_err(),
        "Decryption with wrong password should fail"
    );
    match result {
        Err(KeyError::Decryption(msg)) => {
            assert!(
                msg.contains("wrong password"),
                "Error message should hint at wrong password"
            );
        }
        _ => panic!("Should return Decryption error"),
    }

    // Test 2: Wrong password for secret key decryption
    let (secret_key, _) = derive_f1r3fly_key(&mnemonic).expect("Failed to derive key");
    let encrypted_secret =
        encrypt_secret_key(&secret_key, correct_password).expect("Failed to encrypt secret");

    let result = decrypt_secret_key(&encrypted_secret, wrong_password);
    assert!(
        result.is_err(),
        "Secret key decryption with wrong password should fail"
    );

    // Test 3: Invalid encrypted data (too short)
    let invalid_short = "abcd1234"; // Too short to contain salt + nonce + data
    let result = decrypt_mnemonic(invalid_short, correct_password);
    assert!(result.is_err(), "Decryption of too-short data should fail");
    match result {
        Err(KeyError::Decryption(msg)) => {
            assert!(
                msg.contains("too short") || msg.contains("44 bytes"),
                "Error should mention minimum length requirement"
            );
        }
        _ => panic!("Should return Decryption error"),
    }

    // Test 4: Invalid hex encoding
    let invalid_hex = "not_valid_hex_zzzz";
    let result = decrypt_mnemonic(invalid_hex, correct_password);
    assert!(result.is_err(), "Decryption of invalid hex should fail");

    // Test 5: Valid hex but corrupted data (tampered ciphertext)
    let mut encrypted_bytes = hex::decode(&encrypted).expect("Should decode");
    // Corrupt the last byte (will fail authentication tag verification)
    if let Some(last) = encrypted_bytes.last_mut() {
        *last ^= 0xFF; // Flip all bits
    }
    let corrupted = hex::encode(encrypted_bytes);

    let result = decrypt_mnemonic(&corrupted, correct_password);
    assert!(
        result.is_err(),
        "Decryption of tampered data should fail (GCM authentication)"
    );

    // Test 6: Empty password edge case
    let encrypted_with_empty =
        encrypt_mnemonic(&mnemonic, "").expect("Should encrypt with empty password");
    let result = decrypt_mnemonic(&encrypted_with_empty, "not_empty");
    assert!(
        result.is_err(),
        "Decryption with wrong password (even if encrypted with empty) should fail"
    );

    // Verify it works with correct empty password
    let decrypted = decrypt_mnemonic(&encrypted_with_empty, "")
        .expect("Should decrypt with matching empty password");
    assert_eq!(
        mnemonic.to_string(),
        decrypted.to_string(),
        "Empty password should work if used consistently"
    );
}
