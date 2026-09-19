use std::time::{Duration, Instant};

use crate::transaction::Transaction;

/// Session identifier (nanoid)
pub type SessionId = String;

/// Stable identity bound to a server-side transaction session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPrincipal {
    Anonymous,
    Authenticated(String),
}

/// Immutable owner and database scope of a server-side transaction session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOwner {
    pub principal: SessionPrincipal,
    pub database: String,
}

impl SessionOwner {
    pub fn anonymous(database: impl Into<String>) -> Self {
        Self {
            principal: SessionPrincipal::Anonymous,
            database: database.into(),
        }
    }

    pub fn authenticated(identity: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            principal: SessionPrincipal::Authenticated(identity.into()),
            database: database.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionAccessError {
    #[error("session belongs to a different principal")]
    PrincipalMismatch,
    #[error("session belongs to a different database")]
    DatabaseMismatch,
    #[error("session is unknown, expired, or no longer active")]
    Expired,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("{0}")]
    Access(#[from] SessionAccessError),
    #[error("{0}")]
    Transaction(#[from] crate::transaction::TransactionError),
}

#[derive(Debug)]
pub(crate) enum SessionOperationError<E> {
    Access(SessionAccessError),
    Transaction(crate::transaction::TransactionError),
    Operation(E),
}

/// Session configuration
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Timeout for inactive transactions (default: 5 minutes)
    pub transaction_timeout: Duration,

    /// Interval for cleanup task (default: 30 seconds)
    pub cleanup_interval: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            transaction_timeout: Duration::from_secs(5 * 60),
            cleanup_interval: Duration::from_secs(30),
        }
    }
}

/// A client session that may hold an active transaction
pub struct Session {
    /// Unique session identifier
    pub id: SessionId,

    /// Immutable owner and database scope.
    pub owner: SessionOwner,

    /// Active transaction (if any)
    pub transaction: Option<Transaction>,

    /// Last activity timestamp
    pub last_activity: Instant,

    /// Per-session query timeout (set via SET QUERY_TIMEOUT)
    pub query_timeout: Option<Duration>,
}

impl Session {
    /// Create a new session
    pub fn new(id: SessionId, owner: SessionOwner) -> Self {
        Self {
            id,
            owner,
            transaction: None,
            last_activity: Instant::now(),
            query_timeout: None,
        }
    }

    /// Update last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Check if session has an active transaction
    pub fn has_transaction(&self) -> bool {
        self.transaction
            .as_ref()
            .map(|tx| tx.is_active())
            .unwrap_or(false)
    }

    /// Check if session has timed out
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.last_activity.elapsed() > timeout
    }
}
