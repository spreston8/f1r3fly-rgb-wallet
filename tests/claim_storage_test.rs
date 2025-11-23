//! Tests for ClaimStorage hybrid storage implementation
//!
//! Tests SQLite persistence + in-memory cache for witness claim tracking

use f1r3fly_rgb_wallet::storage::claim_storage::{ClaimStatus, ClaimStorage, PendingClaim};
use std::path::PathBuf;

#[test]
fn test_claim_storage_initialization_creates_schema() {
    // Setup
    let temp_dir = tempfile::tempdir().unwrap();

    // Create storage
    let _storage = ClaimStorage::new(temp_dir.path()).unwrap();

    // Verify database file exists
    let db_path = temp_dir.path().join("f1r3fly_claims.db");
    assert!(db_path.exists(), "Database file should be created");

    // Verify tables exist by querying schema
    // Scope the connection to ensure it's closed before temp_dir cleanup (Windows compatibility)
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();

        // Check pending_claims table exists
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='pending_claims'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_exists, 1, "pending_claims table should exist");

        // Check indexes exist
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_pending_claims_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 3, "Should have 3 indexes on pending_claims");

        // Check consignment_files table exists
        let consignment_table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='consignment_files'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            consignment_table_exists, 1,
            "consignment_files table should exist"
        );
    } // Connection drops here, releasing file lock
}

#[test]
fn test_insert_claim_and_retrieve_with_cache() {
    // Setup
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage = ClaimStorage::new(temp_dir.path()).unwrap();

    // Create test claim
    let claim = PendingClaim {
        id: None,
        witness_id: "witness:a3467636599ef254:0".to_string(),
        recipient_address: "bc1q...xyz".to_string(),
        expected_vout: 0,
        contract_id: "test_contract_123".to_string(),
        consignment_file: PathBuf::from("/tmp/consignment.json"),
        status: ClaimStatus::Pending,
        error: None,
        created_at: 1700000000,
        claimed_at: None,
    };

    // Insert claim (should write to DB and cache)
    let claim_id = storage.insert_pending_claim(&claim).unwrap();
    assert!(claim_id > 0, "Should return valid row ID");

    // Retrieve from cache (fast path)
    let retrieved = storage
        .get_pending_claims(Some("test_contract_123"))
        .unwrap();

    assert_eq!(retrieved.len(), 1, "Should retrieve 1 claim");
    assert_eq!(retrieved[0].witness_id, claim.witness_id);
    assert_eq!(retrieved[0].status, ClaimStatus::Pending);
    assert_eq!(retrieved[0].id, Some(claim_id));

    // Verify data is actually in database (not just cache)
    // Scope the connection to ensure it's closed before temp_dir cleanup (Windows compatibility)
    {
        let conn = rusqlite::Connection::open(temp_dir.path().join("f1r3fly_claims.db")).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM pending_claims", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "Should have 1 row in database");
    } // Connection drops here, releasing file lock
}

#[test]
fn test_cache_invalidation_and_rebuild() {
    // Setup
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage = ClaimStorage::new(temp_dir.path()).unwrap();

    // Insert multiple claims
    for i in 0..3 {
        let claim = PendingClaim {
            id: None,
            witness_id: format!("witness:test{}:0", i),
            recipient_address: format!("bc1q...{}", i),
            expected_vout: 0,
            contract_id: "contract_A".to_string(),
            consignment_file: PathBuf::from(format!("/tmp/consignment_{}.json", i)),
            status: ClaimStatus::Pending,
            error: None,
            created_at: 1700000000 + i,
            claimed_at: None,
        };
        storage.insert_pending_claim(&claim).unwrap();
    }

    // Verify cache is populated (first query)
    let claims_before = storage.get_pending_claims(Some("contract_A")).unwrap();
    assert_eq!(claims_before.len(), 3, "Should have 3 claims in cache");

    // Invalidate cache
    storage.invalidate_cache();

    // Query again - should rebuild from DB
    let claims_after = storage.get_pending_claims(Some("contract_A")).unwrap();
    assert_eq!(claims_after.len(), 3, "Should rebuild cache with 3 claims");

    // Verify data integrity after rebuild
    for (before, after) in claims_before.iter().zip(claims_after.iter()) {
        assert_eq!(before.witness_id, after.witness_id);
        assert_eq!(before.id, after.id);
    }
}

#[test]
fn test_update_claim_status_atomic_transaction() {
    // Setup
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage = ClaimStorage::new(temp_dir.path()).unwrap();

    // Insert claim
    let claim = PendingClaim {
        id: None,
        witness_id: "witness:abc123:0".to_string(),
        recipient_address: "bc1q...xyz".to_string(),
        expected_vout: 0,
        contract_id: "test_contract".to_string(),
        consignment_file: PathBuf::from("/tmp/test.json"),
        status: ClaimStatus::Pending,
        error: None,
        created_at: 1700000000,
        claimed_at: None,
    };

    let claim_id = storage.insert_pending_claim(&claim).unwrap();

    // Update to Claimed status
    storage.mark_claim_completed(claim_id).unwrap();

    // Verify cache was updated
    let claims = storage.get_all_claims("test_contract").unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].status, ClaimStatus::Claimed);
    assert!(claims[0].claimed_at.is_some(), "claimed_at should be set");

    // Verify database was updated
    // Scope the connection to ensure it's closed before temp_dir cleanup (Windows compatibility)
    {
        let conn = rusqlite::Connection::open(temp_dir.path().join("f1r3fly_claims.db")).unwrap();
        let (status, claimed_at): (String, Option<u64>) = conn
            .query_row(
                "SELECT status, claimed_at FROM pending_claims WHERE id = ?1",
                [claim_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(status, "claimed");
        assert!(claimed_at.is_some(), "claimed_at should be set in DB");
    } // Connection drops here, releasing file lock

    // Update to Failed status with error
    storage
        .update_claim_status(
            claim_id,
            ClaimStatus::Failed,
            Some("UTXO not found".to_string()),
        )
        .unwrap();

    // Verify error is stored
    let claims_after_fail = storage.get_all_claims("test_contract").unwrap();
    assert_eq!(claims_after_fail[0].status, ClaimStatus::Failed);
    assert_eq!(
        claims_after_fail[0].error,
        Some("UTXO not found".to_string())
    );
}

#[test]
fn test_multiple_contracts_query_filtering() {
    // Setup
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage = ClaimStorage::new(temp_dir.path()).unwrap();

    // Insert claims for contract A
    for i in 0..3 {
        let claim = PendingClaim {
            id: None,
            witness_id: format!("witness:contractA_{}:0", i),
            recipient_address: format!("bc1q...A{}", i),
            expected_vout: 0,
            contract_id: "contract_A".to_string(),
            consignment_file: PathBuf::from(format!("/tmp/A_{}.json", i)),
            status: ClaimStatus::Pending,
            error: None,
            created_at: 1700000000 + i,
            claimed_at: None,
        };
        storage.insert_pending_claim(&claim).unwrap();
    }

    // Insert claims for contract B
    for i in 0..2 {
        let claim = PendingClaim {
            id: None,
            witness_id: format!("witness:contractB_{}:0", i),
            recipient_address: format!("bc1q...B{}", i),
            expected_vout: 0,
            contract_id: "contract_B".to_string(),
            consignment_file: PathBuf::from(format!("/tmp/B_{}.json", i)),
            status: ClaimStatus::Pending,
            error: None,
            created_at: 1700000000 + i,
            claimed_at: None,
        };
        storage.insert_pending_claim(&claim).unwrap();
    }

    // Query contract A only
    let claims_a = storage.get_pending_claims(Some("contract_A")).unwrap();
    assert_eq!(claims_a.len(), 3, "Should have 3 claims for contract A");
    assert!(claims_a.iter().all(|c| c.contract_id == "contract_A"));

    // Query contract B only
    let claims_b = storage.get_pending_claims(Some("contract_B")).unwrap();
    assert_eq!(claims_b.len(), 2, "Should have 2 claims for contract B");
    assert!(claims_b.iter().all(|c| c.contract_id == "contract_B"));

    // Query all contracts
    let all_claims = storage.get_pending_claims(None).unwrap();
    assert_eq!(all_claims.len(), 5, "Should have 5 total pending claims");

    // Mark one from contract A as claimed
    let claim_a_id = claims_a[0].id.unwrap();
    storage.mark_claim_completed(claim_a_id).unwrap();

    // Verify only pending claims are returned
    let pending_a = storage.get_pending_claims(Some("contract_A")).unwrap();
    assert_eq!(
        pending_a.len(),
        2,
        "Should have 2 pending claims for contract A"
    );

    // Verify claimed claim is in get_all_claims
    let all_a = storage.get_all_claims("contract_A").unwrap();
    assert_eq!(
        all_a.len(),
        3,
        "Should still have 3 total claims for contract A"
    );
    assert_eq!(
        all_a
            .iter()
            .filter(|c| c.status == ClaimStatus::Claimed)
            .count(),
        1,
        "Should have 1 claimed"
    );
}
