/// Represents a pending write operation within a transaction
#[derive(Debug, Clone)]
pub enum PendingWrite {
    /// Insert or update with serialized value bytes
    Insert(Vec<u8>),
    /// Delete marker
    Delete,
}

/// Transaction lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// Transaction is active and accepting operations
    Active,
    /// Transaction was successfully committed
    Committed,
    /// Transaction was rolled back (explicit or due to error)
    RolledBack,
}
