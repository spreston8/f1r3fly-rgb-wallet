//! Hybrid storage for witness claim tracking
//!
//! Combines SQLite (durable persistence) with in-memory cache (fast reads)
//! for optimal performance and reliability.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Status of a witness claim
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClaimStatus {
    /// Claim is pending (UTXO not found yet or claim not executed)
    Pending,
    /// Claim was successfully executed
    Claimed,
    /// Claim failed (will retry on next sync)
    Failed,
}

impl ClaimStatus {
    /// Convert to database string representation
    fn to_db_string(&self) -> &'static str {
        match self {
            ClaimStatus::Pending => "pending",
            ClaimStatus::Claimed => "claimed",
            ClaimStatus::Failed => "failed",
        }
    }

    /// Parse from database string
    fn from_db_string(s: &str) -> Result<Self, StorageError> {
        match s {
            "pending" => Ok(ClaimStatus::Pending),
            "claimed" => Ok(ClaimStatus::Claimed),
            "failed" => Ok(ClaimStatus::Failed),
            _ => Err(StorageError::InvalidData(format!(
                "Invalid claim status: {}",
                s
            ))),
        }
    }
}

/// Pending claim entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingClaim {
    /// Database ID (None if not yet inserted)
    pub id: Option<i64>,

    /// Witness identifier (e.g. "witness:a3467636:0")
    pub witness_id: String,

    /// Recipient's Bitcoin address (from invoice, pre-Tapret)
    pub recipient_address: String,

    /// Expected vout in the Bitcoin transaction
    pub expected_vout: u32,

    /// Contract ID
    pub contract_id: String,

    /// Path to consignment file (source of truth)
    pub consignment_file: PathBuf,

    /// Current status
    pub status: ClaimStatus,

    /// Error message (if status is Failed)
    pub error: Option<String>,

    /// Unix timestamp when claim was created
    pub created_at: u64,

    /// Unix timestamp when claim was completed (if status is Claimed)
    pub claimed_at: Option<u64>,

    /// Actual Bitcoin TXID (from consignment witness_tx, post-Tapret)
    /// This is the REAL on-chain UTXO, not from wallet discovery
    pub actual_txid: Option<String>,

    /// Actual vout (from consignment witness_tx)
    pub actual_vout: Option<u32>,
}

/// Storage errors
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

/// Hybrid storage with SQLite persistence + in-memory cache
pub struct ClaimStorage {
    /// SQLite connection for durable persistence
    conn: Connection,

    /// In-memory cache: contract_id → Vec<PendingClaim>
    /// Read-mostly workload, RwLock allows concurrent reads
    cache: RwLock<HashMap<String, Vec<PendingClaim>>>,
}

impl ClaimStorage {
    /// Create new storage with hybrid architecture
    ///
    /// # Arguments
    ///
    /// * `wallet_dir` - Wallet directory path (database will be created here)
    ///
    /// # Returns
    ///
    /// New ClaimStorage instance with initialized schema
    pub fn new<P: AsRef<Path>>(wallet_dir: P) -> Result<Self, StorageError> {
        let db_path = wallet_dir.as_ref().join("f1r3fly_claims.db");

        log::info!("Opening claims database: {}", db_path.display());

        let conn = Connection::open(&db_path)?;

        // Initialize schema
        Self::init_schema(&conn)?;

        // Initialize empty cache (populated on first query)
        let cache = RwLock::new(HashMap::new());

        Ok(Self { conn, cache })
    }

    /// Initialize database schema
    fn init_schema(conn: &Connection) -> Result<(), StorageError> {
        // Pending claims table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pending_claims (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                witness_id TEXT NOT NULL,
                recipient_address TEXT NOT NULL,
                expected_vout INTEGER NOT NULL,
                contract_id TEXT NOT NULL,
                consignment_file TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('pending', 'claimed', 'failed')),
                error TEXT,
                created_at INTEGER NOT NULL,
                claimed_at INTEGER,
                actual_txid TEXT,
                actual_vout INTEGER,
                UNIQUE(witness_id, contract_id)
            )",
            [],
        )?;

        // Migration: Add actual_txid and actual_vout columns if they don't exist
        // This is idempotent and safe to run on existing databases
        let _ = conn.execute("ALTER TABLE pending_claims ADD COLUMN actual_txid TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE pending_claims ADD COLUMN actual_vout INTEGER",
            [],
        );

        // Indexes for fast queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_claims_status ON pending_claims(status)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_claims_contract ON pending_claims(contract_id)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_claims_created ON pending_claims(created_at)",
            [],
        )?;

        // Consignment files tracking table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS consignment_files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                contract_id TEXT NOT NULL,
                file_path TEXT NOT NULL UNIQUE,
                is_genesis BOOLEAN NOT NULL,
                accepted_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_consignment_contract ON consignment_files(contract_id)",
            [],
        )?;

        log::debug!("✓ Database schema initialized");

        Ok(())
    }

    /// Insert claim with write-through to both DB and cache
    ///
    /// # Arguments
    ///
    /// * `claim` - Claim to insert
    ///
    /// # Returns
    ///
    /// Database row ID of inserted claim
    pub fn insert_pending_claim(&mut self, claim: &PendingClaim) -> Result<i64, StorageError> {
        // Start transaction for atomicity
        let tx = self.conn.transaction()?;

        // Write to SQLite (durable)
        tx.execute(
            "INSERT INTO pending_claims (witness_id, recipient_address, expected_vout, 
             contract_id, consignment_file, status, error, created_at, claimed_at, actual_txid, actual_vout)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                &claim.witness_id,
                &claim.recipient_address,
                claim.expected_vout,
                &claim.contract_id,
                claim.consignment_file.to_str(),
                claim.status.to_db_string(),
                &claim.error,
                claim.created_at,
                claim.claimed_at,
                &claim.actual_txid,
                claim.actual_vout,
            ],
        )?;

        let row_id = tx.last_insert_rowid();

        // Commit transaction
        tx.commit()?;

        log::debug!("✓ Inserted claim to DB (id={})", row_id);

        // Update cache (fast reads)
        let mut cache = self.cache.write().unwrap();
        let mut claim_with_id = claim.clone();
        claim_with_id.id = Some(row_id);

        cache
            .entry(claim.contract_id.clone())
            .or_insert_with(Vec::new)
            .push(claim_with_id);

        log::debug!("✓ Updated cache for contract {}", claim.contract_id);

        Ok(row_id)
    }

    /// Query claims with cache-first strategy
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Optional contract ID to filter by
    ///
    /// # Returns
    ///
    /// Vector of pending claims (status = Pending)
    pub fn get_pending_claims(
        &self,
        contract_id: Option<&str>,
    ) -> Result<Vec<PendingClaim>, StorageError> {
        // Fast path: Try cache first (0.01ms)
        if let Some(cid) = contract_id {
            let cache = self.cache.read().unwrap();
            if let Some(claims) = cache.get(cid) {
                // Cache hit! Return pending claims only
                return Ok(claims
                    .iter()
                    .filter(|c| c.status == ClaimStatus::Pending)
                    .cloned()
                    .collect());
            }
        }

        // Cache miss or full query: Read from SQLite (0.5-2ms)
        let claims = self.query_database(contract_id, Some(ClaimStatus::Pending))?;

        // Populate cache for future queries (only for specific contract queries)
        // Don't populate cache for None queries to avoid duplication
        if let Some(cid) = contract_id {
            let mut cache = self.cache.write().unwrap();
            // Replace existing cache entry (don't append)
            cache.insert(cid.to_string(), claims.clone());
        }

        Ok(claims)
    }

    /// Update claim status with write-through
    ///
    /// # Arguments
    ///
    /// * `id` - Database row ID
    /// * `status` - New status
    /// * `error` - Optional error message (for Failed status)
    pub fn update_claim_status(
        &mut self,
        id: i64,
        status: ClaimStatus,
        error: Option<String>,
    ) -> Result<(), StorageError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Start transaction
        let tx = self.conn.transaction()?;

        // Write to SQLite
        let claimed_at = if status == ClaimStatus::Claimed {
            Some(now)
        } else {
            None
        };

        tx.execute(
            "UPDATE pending_claims SET status = ?1, error = ?2, claimed_at = ?3 WHERE id = ?4",
            params![status.to_db_string(), error, claimed_at, id],
        )?;

        tx.commit()?;

        log::debug!("✓ Updated claim status in DB (id={})", id);

        // Update cache
        let mut cache = self.cache.write().unwrap();
        for claims in cache.values_mut() {
            if let Some(claim) = claims.iter_mut().find(|c| c.id == Some(id)) {
                claim.status = status;
                claim.error = error;
                claim.claimed_at = claimed_at;
                log::debug!("✓ Updated claim in cache (id={})", id);
                break;
            }
        }

        Ok(())
    }

    /// Mark claim as completed with write-through
    ///
    /// Convenience method for updating status to Claimed
    ///
    /// # Arguments
    ///
    /// * `id` - Database row ID
    pub fn mark_claim_completed(&mut self, id: i64) -> Result<(), StorageError> {
        self.update_claim_status(id, ClaimStatus::Claimed, None)
    }

    /// Invalidate cache (called on wallet load to rebuild from DB)
    pub fn invalidate_cache(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
        log::debug!("✓ Cache invalidated (will rebuild on next query)");
    }

    /// Get all claims for a contract (for debugging/admin)
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID
    ///
    /// # Returns
    ///
    /// All claims (any status) for the contract
    pub fn get_all_claims(&self, contract_id: &str) -> Result<Vec<PendingClaim>, StorageError> {
        self.query_database(Some(contract_id), None)
    }

    /// Get all successfully claimed UTXOs for a contract
    ///
    /// Returns actual txid:vout pairs for all claimed RGB transfers.
    /// These UTXOs need to be included in balance queries even if BDK doesn't track them
    /// (because Tapret commitments modify the address).
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID
    ///
    /// # Returns
    ///
    /// Vector of (txid, vout) tuples for claimed UTXOs
    pub fn get_claimed_utxos(&self, contract_id: &str) -> Result<Vec<(String, u32)>, StorageError> {
        let claimed = self.query_database(Some(contract_id), Some(ClaimStatus::Claimed))?;

        let mut utxos = Vec::new();
        for claim in claimed {
            if let (Some(txid), Some(vout)) = (claim.actual_txid, claim.actual_vout) {
                utxos.push((txid, vout));
            }
        }

        Ok(utxos)
    }

    /// Internal: Query database
    fn query_database(
        &self,
        contract_id: Option<&str>,
        status_filter: Option<ClaimStatus>,
    ) -> Result<Vec<PendingClaim>, StorageError> {
        let mut query = "SELECT id, witness_id, recipient_address, expected_vout, contract_id, 
                         consignment_file, status, error, created_at, claimed_at, actual_txid, actual_vout 
                         FROM pending_claims WHERE 1=1"
            .to_string();

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(cid) = contract_id {
            query.push_str(" AND contract_id = ?");
            params_vec.push(Box::new(cid.to_string()));
        }

        if let Some(status) = &status_filter {
            query.push_str(" AND status = ?");
            params_vec.push(Box::new(status.to_db_string().to_string()));
        }

        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec
            .iter()
            .map(|p| &**p as &dyn rusqlite::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&query)?;
        let claims = stmt.query_map(&params_refs[..], |row| {
            Ok(PendingClaim {
                id: Some(row.get(0)?),
                witness_id: row.get(1)?,
                recipient_address: row.get(2)?,
                expected_vout: row.get(3)?,
                contract_id: row.get(4)?,
                consignment_file: PathBuf::from(row.get::<_, String>(5)?),
                status: ClaimStatus::from_db_string(&row.get::<_, String>(6)?)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                error: row.get(7)?,
                created_at: row.get(8)?,
                claimed_at: row.get(9)?,
                actual_txid: row.get(10)?,
                actual_vout: row.get(11)?,
            })
        })?;

        let result: Result<Vec<_>, _> = claims.collect();
        Ok(result?)
    }

    /// Track consignment file acceptance
    ///
    /// Records when a consignment file was accepted for recovery purposes
    ///
    /// # Arguments
    ///
    /// * `contract_id` - Contract ID
    /// * `file_path` - Path to consignment file
    /// * `is_genesis` - Whether this is a genesis consignment
    pub fn track_consignment_file(
        &mut self,
        contract_id: &str,
        file_path: &Path,
        is_genesis: bool,
    ) -> Result<(), StorageError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.conn.execute(
            "INSERT OR IGNORE INTO consignment_files (contract_id, file_path, is_genesis, accepted_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![contract_id, file_path.to_str(), is_genesis, now],
        )?;

        Ok(())
    }
}
