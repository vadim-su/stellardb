//! Namespace - manages multiple databases within a data directory
//!
//! Each database is an isolated fjall instance in its own subdirectory.
//! The filesystem is the source of truth for which databases exist.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::StorageError;
use crate::storage::{Database, IndexBuildConfig};

/// Subdirectory containing all databases
const DATABASES_DIR: &str = "databases";

pub struct Namespace {
    data_dir: PathBuf,
    databases_dir: PathBuf,
    databases: RwLock<HashMap<String, Arc<Database>>>,
    /// Set of known database names (from FS scan at startup).
    /// Acquire this lock BEFORE `databases` to avoid deadlock.
    known: RwLock<HashSet<String>>,
    index_build_config: IndexBuildConfig,
}

// Manual Debug impl — Database doesn't derive Debug
impl std::fmt::Debug for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Namespace")
            .field("data_dir", &self.data_dir)
            .field("databases_dir", &self.databases_dir)
            .field("known", &self.known.read().len())
            .finish()
    }
}

impl Namespace {
    /// Open a namespace at the given data directory.
    ///
    /// Scans `data_dir/databases/` to discover existing databases.
    /// Does NOT open fjall instances — they are opened lazily on first access.
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        Self::open_with_index_build_config(data_dir, IndexBuildConfig::default())
    }

    pub fn open_with_index_build_config(
        data_dir: &Path,
        index_build_config: IndexBuildConfig,
    ) -> Result<Self, StorageError> {
        let index_build_config = index_build_config.validate()?;
        let databases_dir = data_dir.join(DATABASES_DIR);
        std::fs::create_dir_all(&databases_dir).map_err(|e| {
            StorageError::Io(format!("Failed to create databases directory: {}", e))
        })?;

        // Scan filesystem for existing databases
        let mut known = HashSet::new();
        for entry in std::fs::read_dir(&databases_dir)
            .map_err(|e| StorageError::Io(format!("Failed to read databases directory: {}", e)))?
        {
            let entry = entry
                .map_err(|e| StorageError::Io(format!("Failed to read directory entry: {}", e)))?;
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    known.insert(name.to_string());
                } else {
                    tracing::warn!("Skipping non-UTF-8 database directory: {:?}", entry.path());
                }
            }
        }

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            databases_dir,
            databases: RwLock::new(HashMap::new()),
            known: RwLock::new(known),
            index_build_config,
        })
    }

    /// Get the data directory path.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Get or lazily open a database by name.
    pub fn get_database(&self, name: &str) -> Result<Arc<Database>, StorageError> {
        // Fast path: already open
        if let Some(db) = self.databases.read().get(name) {
            return Ok(db.clone());
        }

        // Validate name to prevent path traversal
        crate::schema::validate_name("database", name)
            .map_err(|e| StorageError::InvalidOperation(e.to_string()))?;

        // Check if database exists on disk
        if !self.known.read().contains(name) {
            return Err(StorageError::NotFound(format!(
                "Database '{}' not found",
                name
            )));
        }

        // Slow path: open and cache
        let mut databases = self.databases.write();

        // Double-check after acquiring write lock
        if let Some(db) = databases.get(name) {
            return Ok(db.clone());
        }

        let db_path = self.databases_dir.join(name);
        let db = Database::open(&db_path)?;
        db.set_index_build_config(self.index_build_config)?;
        let db = Arc::new(db);
        databases.insert(name.to_string(), db.clone());
        Ok(db)
    }

    /// Create a new database.
    pub fn create_database(&self, name: &str) -> Result<Arc<Database>, StorageError> {
        crate::schema::validate_name("database", name)
            .map_err(|e| StorageError::InvalidOperation(e.to_string()))?;

        // Hold write lock on `known` for the entire operation to prevent races
        let mut known = self.known.write();
        if known.contains(name) {
            return Err(StorageError::AlreadyExists(format!("Database '{}'", name)));
        }

        let db_path = self.databases_dir.join(name);
        std::fs::create_dir_all(&db_path)
            .map_err(|e| StorageError::Io(format!("Failed to create database directory: {}", e)))?;

        let db = Database::open(&db_path)?;
        db.set_index_build_config(self.index_build_config)?;
        let db = Arc::new(db);

        known.insert(name.to_string());
        self.databases.write().insert(name.to_string(), db.clone());

        Ok(db)
    }

    /// Drop a database. Removes from cache and deletes directory.
    pub fn drop_database(&self, name: &str) -> Result<(), StorageError> {
        // Validate name to prevent path traversal
        crate::schema::validate_name("database", name)
            .map_err(|e| StorageError::InvalidOperation(e.to_string()))?;

        if name == "_system" {
            return Err(StorageError::InvalidOperation(
                "Cannot drop system database '_system'".into(),
            ));
        }

        // Hold write lock on `known` for the entire operation to prevent races
        let mut known = self.known.write();
        if !known.contains(name) {
            return Err(StorageError::NotFound(format!(
                "Database '{}' not found",
                name
            )));
        }

        // Remove from cache (drops Arc — fjall closes when refcount hits 0)
        self.databases.write().remove(name);
        known.remove(name);

        // Delete directory
        let db_path = self.databases_dir.join(name);
        std::fs::remove_dir_all(&db_path)
            .map_err(|e| StorageError::Io(format!("Failed to delete database directory: {}", e)))?;

        Ok(())
    }

    /// List all database names.
    pub fn list_databases(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .known
            .read()
            .iter()
            .filter(|name| name.as_str() != "_system")
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// Check if a database exists.
    pub fn database_exists(&self, name: &str) -> bool {
        self.known.read().contains(name)
    }

    /// Ensure the _system database exists (create if missing).
    pub fn ensure_system_database(&self) -> Result<Arc<Database>, StorageError> {
        if self.database_exists("_system") {
            self.get_database("_system")
        } else {
            self.create_database("_system")
        }
    }

    /// Create a portable filesystem backup of the entire namespace data directory.
    pub fn backup_to(&self, destination: &Path) -> Result<(), StorageError> {
        if destination.starts_with(&self.data_dir) {
            return Err(StorageError::InvalidOperation(
                "Backup destination must not be inside the source data directory".into(),
            ));
        }
        ensure_empty_or_missing_dir(destination, "backup destination")?;

        let open_databases: Vec<_> = self.databases.read().values().cloned().collect();
        for db in open_databases {
            db.sync()?;
        }

        copy_dir_all(&self.data_dir, destination)
    }

    /// Restore a namespace backup into a new or empty data directory.
    pub fn restore_from(backup: &Path, destination: &Path) -> Result<(), StorageError> {
        if !backup.is_dir() {
            return Err(StorageError::NotFound(format!(
                "Backup directory '{}' not found",
                backup.display()
            )));
        }
        ensure_empty_or_missing_dir(destination, "restore destination")?;
        copy_dir_all(backup, destination)
    }
}

fn ensure_empty_or_missing_dir(path: &Path, label: &str) -> Result<(), StorageError> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        return Err(StorageError::InvalidOperation(format!(
            "{} '{}' is not a directory",
            label,
            path.display()
        )));
    }
    if std::fs::read_dir(path)
        .map_err(|e| {
            StorageError::Io(format!(
                "Failed to read {} '{}': {}",
                label,
                path.display(),
                e
            ))
        })?
        .next()
        .is_some()
    {
        return Err(StorageError::InvalidOperation(format!(
            "{} '{}' must be empty",
            label,
            path.display()
        )));
    }
    Ok(())
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<(), StorageError> {
    std::fs::create_dir_all(destination).map_err(|e| {
        StorageError::Io(format!(
            "Failed to create directory '{}': {}",
            destination.display(),
            e
        ))
    })?;

    for entry in std::fs::read_dir(source).map_err(|e| {
        StorageError::Io(format!(
            "Failed to read directory '{}': {}",
            source.display(),
            e
        ))
    })? {
        let entry = entry
            .map_err(|e| StorageError::Io(format!("Failed to read directory entry: {}", e)))?;
        let file_type = entry.file_type().map_err(|e| {
            StorageError::Io(format!(
                "Failed to read file type for '{}': {}",
                entry.path().display(),
                e
            ))
        })?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target).map_err(|e| {
                StorageError::Io(format!(
                    "Failed to copy '{}' to '{}': {}",
                    entry.path().display(),
                    target.display(),
                    e
                ))
            })?;
        } else {
            return Err(StorageError::InvalidOperation(format!(
                "Cannot back up unsupported filesystem entry '{}'",
                entry.path().display()
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_open_empty_namespace() {
        let dir = TempDir::new().unwrap();
        let ns = Namespace::open(dir.path()).unwrap();
        assert!(ns.list_databases().is_empty());
    }

    #[test]
    fn test_create_and_get_database() {
        let dir = TempDir::new().unwrap();
        let ns = Namespace::open(dir.path()).unwrap();

        let db = ns.create_database("testdb").unwrap();
        assert!(ns.database_exists("testdb"));

        // Should be able to get it again
        let db2 = ns.get_database("testdb").unwrap();
        assert!(Arc::ptr_eq(&db, &db2));
    }

    #[test]
    fn test_create_duplicate_database() {
        let dir = TempDir::new().unwrap();
        let ns = Namespace::open(dir.path()).unwrap();

        ns.create_database("mydb").unwrap();
        match ns.create_database("mydb") {
            Err(e) => assert!(e.to_string().contains("already exists")),
            Ok(_) => panic!("expected error for duplicate database"),
        }
    }

    #[test]
    fn test_get_nonexistent_database() {
        let dir = TempDir::new().unwrap();
        let ns = Namespace::open(dir.path()).unwrap();

        match ns.get_database("nope") {
            Err(e) => assert!(e.to_string().contains("not found")),
            Ok(_) => panic!("expected error for nonexistent database"),
        }
    }

    #[test]
    fn test_drop_database() {
        let dir = TempDir::new().unwrap();
        let ns = Namespace::open(dir.path()).unwrap();

        ns.create_database("dropme").unwrap();
        assert!(ns.database_exists("dropme"));

        ns.drop_database("dropme").unwrap();
        assert!(!ns.database_exists("dropme"));
    }

    #[test]
    fn test_list_databases_sorted() {
        let dir = TempDir::new().unwrap();
        let ns = Namespace::open(dir.path()).unwrap();

        ns.create_database("charlie").unwrap();
        ns.create_database("alpha").unwrap();
        ns.create_database("bravo").unwrap();
        ns.ensure_system_database().unwrap();

        assert_eq!(ns.list_databases(), vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn test_cannot_drop_system_database() {
        let dir = TempDir::new().unwrap();
        let ns = Namespace::open(dir.path()).unwrap();

        ns.create_database("_system").unwrap();
        assert!(ns.database_exists("_system"));

        match ns.drop_database("_system") {
            Err(e) => assert!(
                e.to_string().contains("Cannot drop system database"),
                "unexpected error: {}",
                e
            ),
            Ok(_) => panic!("expected error when dropping _system"),
        }

        // Verify it still exists
        assert!(ns.database_exists("_system"));
    }

    #[test]
    fn test_lazy_open_persisted_database() {
        let dir = TempDir::new().unwrap();

        // Create a database, then re-open the namespace
        {
            let ns = Namespace::open(dir.path()).unwrap();
            let _db = ns.create_database("persistent").unwrap();
        }

        // Re-open namespace — should discover "persistent" on disk
        let ns = Namespace::open(dir.path()).unwrap();
        assert!(ns.database_exists("persistent"));
        assert_eq!(ns.list_databases(), vec!["persistent"]);

        // Lazy open should work
        let _db = ns.get_database("persistent").unwrap();
    }

    #[test]
    fn test_backup_and_restore_namespace() {
        let source_dir = TempDir::new().unwrap();
        let backup_dir = TempDir::new().unwrap();
        let restore_dir = TempDir::new().unwrap();
        let backup_path = backup_dir.path().join("stellardb-backup");
        let restore_path = restore_dir.path().join("restored-data");

        {
            let ns = Namespace::open(source_dir.path()).unwrap();
            let db = ns.create_database("default").unwrap();
            db.get_or_create_collection("items").unwrap();
            db.create_document(&crate::document::Document {
                id: "items:one".to_string(),
                fields: [(
                    "name".to_string(),
                    crate::document::Value::String("backed up".to_string()),
                )]
                .into_iter()
                .collect(),
            })
            .unwrap();

            ns.backup_to(&backup_path).unwrap();
        }

        Namespace::restore_from(&backup_path, &restore_path).unwrap();
        let restored = Namespace::open(&restore_path).unwrap();
        let db = restored.get_database("default").unwrap();
        let doc = db.get_document("items", "one").unwrap().unwrap();
        assert_eq!(
            doc.fields.get("name"),
            Some(&crate::document::Value::String("backed up".to_string()))
        );
    }
}
