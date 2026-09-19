use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

use crate::storage::Database;
use crate::transaction::{Transaction, TransactionError};

use super::types::{
    Session, SessionAccessError, SessionConfig, SessionError, SessionId, SessionOperationError,
    SessionOwner,
};

/// Manages active sessions
pub struct SessionManager {
    sessions: RwLock<HashMap<SessionId, Session>>,
    config: SessionConfig,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(config: SessionConfig) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            config,
        }
    }

    /// Atomically create and bind a new session to an immutable owner.
    pub fn begin_transaction(&self, storage: Arc<Database>, owner: SessionOwner) -> SessionId {
        let mut sessions = self.sessions.write();
        loop {
            let session_id: SessionId = nanoid::nanoid!();
            if sessions.contains_key(&session_id) {
                continue;
            }

            let tx_id = format!("tx_{}", nanoid::nanoid!());
            let tx = Transaction::new(tx_id, storage);
            let mut session = Session::new(session_id.clone(), owner);
            session.transaction = Some(tx);
            sessions.insert(session_id.clone(), session);
            return session_id;
        }
    }

    fn validate_owner(session: &Session, owner: &SessionOwner) -> Result<(), SessionAccessError> {
        if session.owner.principal != owner.principal {
            return Err(SessionAccessError::PrincipalMismatch);
        }
        if session.owner.database != owner.database {
            return Err(SessionAccessError::DatabaseMismatch);
        }
        Ok(())
    }

    /// Validate an existing active session under the manager lock.
    pub fn validate_access(
        &self,
        session_id: &SessionId,
        owner: &SessionOwner,
    ) -> Result<(), SessionAccessError> {
        let sessions = self.sessions.read();
        let session = sessions
            .get(session_id)
            .ok_or(SessionAccessError::Expired)?;
        Self::validate_owner(session, owner)?;
        if !session.has_transaction() {
            return Err(SessionAccessError::Expired);
        }
        Ok(())
    }

    /// Get mutable access to a session's transaction
    pub fn with_transaction<F, R>(
        &self,
        session_id: &SessionId,
        owner: &SessionOwner,
        f: F,
    ) -> Result<R, SessionError>
    where
        F: FnOnce(&mut Transaction) -> Result<R, TransactionError>,
    {
        let mut sessions = self.sessions.write();
        let session = sessions
            .get_mut(session_id)
            .ok_or(TransactionError::NoActiveTransaction)?;
        Self::validate_owner(session, owner)?;

        session.touch();

        let tx = session
            .transaction
            .as_mut()
            .ok_or(TransactionError::NoActiveTransaction)?;

        if !tx.is_active() {
            return Err(TransactionError::AlreadyCompleted.into());
        }

        Ok(f(tx)?)
    }

    /// Run an operation against a session transaction without erasing its error type.
    pub(crate) fn with_transaction_operation<F, R, E>(
        &self,
        session_id: &SessionId,
        owner: &SessionOwner,
        f: F,
    ) -> Result<R, SessionOperationError<E>>
    where
        F: FnOnce(&mut Transaction) -> Result<R, E>,
    {
        let mut sessions = self.sessions.write();
        let session = sessions.get_mut(session_id).ok_or_else(|| {
            SessionOperationError::Transaction(TransactionError::NoActiveTransaction)
        })?;
        Self::validate_owner(session, owner).map_err(SessionOperationError::Access)?;

        session.touch();

        let tx = session.transaction.as_mut().ok_or_else(|| {
            SessionOperationError::Transaction(TransactionError::NoActiveTransaction)
        })?;

        if !tx.is_active() {
            return Err(SessionOperationError::Transaction(
                TransactionError::AlreadyCompleted,
            ));
        }

        f(tx).map_err(SessionOperationError::Operation)
    }

    /// Commit a session's transaction
    pub fn commit(&self, session_id: &SessionId, owner: &SessionOwner) -> Result<(), SessionError> {
        let mut sessions = self.sessions.write();
        let session = sessions
            .get_mut(session_id)
            .ok_or(TransactionError::NoActiveTransaction)?;
        Self::validate_owner(session, owner)?;

        let tx = session
            .transaction
            .as_mut()
            .ok_or(TransactionError::NoActiveTransaction)?;

        tx.commit()?;

        // Remove session after commit
        sessions.remove(session_id);
        Ok(())
    }

    /// Rollback a session's transaction
    pub fn rollback(
        &self,
        session_id: &SessionId,
        owner: &SessionOwner,
    ) -> Result<(), SessionError> {
        let mut sessions = self.sessions.write();
        let session = sessions
            .get_mut(session_id)
            .ok_or(TransactionError::NoActiveTransaction)?;
        Self::validate_owner(session, owner)?;

        let tx = session
            .transaction
            .as_mut()
            .ok_or(TransactionError::NoActiveTransaction)?;

        tx.rollback()?;

        // Remove session after rollback
        sessions.remove(session_id);
        Ok(())
    }

    /// Check if a session exists and has an active transaction
    pub fn has_active_transaction(
        &self,
        session_id: &SessionId,
        owner: &SessionOwner,
    ) -> Result<bool, SessionAccessError> {
        self.validate_access(session_id, owner).map(|()| true)
    }

    /// Cleanup timed out sessions (call from background task)
    pub fn cleanup_timed_out(&self) -> usize {
        let mut sessions = self.sessions.write();
        let timeout = self.config.transaction_timeout;

        let to_remove: Vec<SessionId> = sessions
            .iter()
            .filter(|(_, session)| session.is_timed_out(timeout))
            .map(|(id, _)| id.clone())
            .collect();

        let count = to_remove.len();

        for id in to_remove {
            if let Some(mut session) = sessions.remove(&id)
                && let Some(tx) = session.transaction.as_mut()
            {
                let _ = tx.rollback();
                tracing::warn!("Transaction timeout, auto rollback: {}", id);
            }
        }

        count
    }

    /// Set query timeout for a session
    pub fn set_query_timeout(
        &self,
        session_id: &SessionId,
        owner: &SessionOwner,
        timeout: Option<Duration>,
    ) -> Result<(), SessionAccessError> {
        let mut sessions = self.sessions.write();
        let session = sessions
            .get_mut(session_id)
            .ok_or(SessionAccessError::Expired)?;
        Self::validate_owner(session, owner)?;
        if !session.has_transaction() {
            return Err(SessionAccessError::Expired);
        }
        session.query_timeout = timeout;
        Ok(())
    }

    /// Get query timeout for a session
    pub fn get_query_timeout(
        &self,
        session_id: &str,
        owner: &SessionOwner,
    ) -> Result<Option<Duration>, SessionAccessError> {
        let sessions = self.sessions.read();
        let session = sessions
            .get(session_id)
            .ok_or(SessionAccessError::Expired)?;
        Self::validate_owner(session, owner)?;
        if !session.has_transaction() {
            return Err(SessionAccessError::Expired);
        }
        Ok(session.query_timeout)
    }

    /// Get configuration
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// Get number of active sessions
    pub fn active_count(&self) -> usize {
        self.sessions.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Arc<Database>) {
        let dir = TempDir::new().unwrap();
        let storage = Arc::new(Database::open(dir.path()).unwrap());
        (dir, storage)
    }

    #[test]
    fn test_session_manager_begin_and_commit() {
        let (_dir, storage) = setup();
        let manager = SessionManager::new(SessionConfig::default());

        let owner = SessionOwner::anonymous("default");
        let session_id = manager.begin_transaction(storage.clone(), owner.clone());
        assert!(manager.has_active_transaction(&session_id, &owner).unwrap());

        // Insert document
        manager
            .with_transaction(&session_id, &owner, |tx| {
                let doc = crate::Document {
                    id: "user:test".to_string(),
                    fields: Default::default(),
                };
                tx.insert(doc)
            })
            .unwrap();

        // Commit
        manager.commit(&session_id, &owner).unwrap();

        // Session should be gone
        assert_eq!(
            manager.has_active_transaction(&session_id, &owner),
            Err(SessionAccessError::Expired)
        );

        // Document should be persisted
        let doc = storage.get_document("user", "test").unwrap();
        assert!(doc.is_some());
    }

    #[test]
    fn session_owner_and_database_are_immutable() {
        let (_dir, storage) = setup();
        let manager = SessionManager::new(SessionConfig::default());
        let alice_db1 = SessionOwner::authenticated("alice-id", "db1");
        let alice_db2 = SessionOwner::authenticated("alice-id", "db2");
        let bob_db1 = SessionOwner::authenticated("bob-id", "db1");
        let root_db1 = SessionOwner::authenticated("root-id", "db1");
        let session_id = manager.begin_transaction(storage, alice_db1.clone());

        manager.validate_access(&session_id, &alice_db1).unwrap();
        assert_eq!(
            manager.validate_access(&session_id, &bob_db1),
            Err(SessionAccessError::PrincipalMismatch)
        );
        assert_eq!(
            manager.validate_access(&session_id, &root_db1),
            Err(SessionAccessError::PrincipalMismatch)
        );
        assert_eq!(
            manager.validate_access(&session_id, &alice_db2),
            Err(SessionAccessError::DatabaseMismatch)
        );

        assert!(matches!(
            manager.commit(&session_id, &bob_db1),
            Err(SessionError::Access(SessionAccessError::PrincipalMismatch))
        ));
        manager.rollback(&session_id, &alice_db1).unwrap();
        assert_eq!(
            manager.validate_access(&session_id, &alice_db1),
            Err(SessionAccessError::Expired)
        );
    }

    #[test]
    fn timeout_cleanup_removes_session_and_owner_binding() {
        let (_dir, storage) = setup();
        let manager = SessionManager::new(SessionConfig {
            transaction_timeout: Duration::ZERO,
            cleanup_interval: Duration::from_secs(1),
        });
        let owner = SessionOwner::authenticated("alice-id", "db1");
        let session_id = manager.begin_transaction(storage, owner.clone());
        assert_eq!(manager.cleanup_timed_out(), 1);
        assert_eq!(
            manager.validate_access(&session_id, &owner),
            Err(SessionAccessError::Expired)
        );
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn cleanup_cannot_interleave_with_transaction_operation() {
        use std::sync::{Barrier, mpsc};

        let (_dir, storage) = setup();
        let manager = Arc::new(SessionManager::new(SessionConfig {
            transaction_timeout: Duration::ZERO,
            cleanup_interval: Duration::from_secs(1),
        }));
        let owner = SessionOwner::anonymous("default");
        let session_id = manager.begin_transaction(storage.clone(), owner.clone());
        let entered = Arc::new(Barrier::new(2));
        let (attempting_tx, attempting_rx) = mpsc::channel();

        let cleanup_manager = manager.clone();
        let cleanup_entered = entered.clone();
        let cleanup = std::thread::spawn(move || {
            cleanup_entered.wait();
            attempting_tx.send(()).unwrap();
            cleanup_manager.cleanup_timed_out()
        });

        manager
            .with_transaction(&session_id, &owner, {
                let entered = entered.clone();
                move |tx| {
                    entered.wait();
                    attempting_rx.recv().unwrap();
                    tx.insert(crate::Document {
                        id: "user:atomic_cleanup".to_string(),
                        fields: Default::default(),
                    })
                }
            })
            .unwrap();

        assert_eq!(cleanup.join().unwrap(), 1);
        assert!(
            storage
                .get_document("user", "atomic_cleanup")
                .unwrap()
                .is_none()
        );
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn test_session_manager_rollback() {
        let (_dir, storage) = setup();
        let manager = SessionManager::new(SessionConfig::default());

        let owner = SessionOwner::anonymous("default");
        let session_id = manager.begin_transaction(storage.clone(), owner.clone());

        manager
            .with_transaction(&session_id, &owner, |tx| {
                let doc = crate::Document {
                    id: "user:rollback_test".to_string(),
                    fields: Default::default(),
                };
                tx.insert(doc)
            })
            .unwrap();

        manager.rollback(&session_id, &owner).unwrap();

        // Document should NOT be persisted
        let doc = storage.get_document("user", "rollback_test").unwrap();
        assert!(doc.is_none());
    }
}
