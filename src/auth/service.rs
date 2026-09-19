//! Central authentication and authorization service for StellarDB.
//!
//! `AuthService` ties together JWT tokens, password hashing, API keys, and the
//! ABAC policy engine. It uses the `_system` database for persistent storage
//! of users, API keys, and policies.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use rand::RngExt;

use crate::document::{Document, Value};
use crate::error::StorageError;
use crate::storage::{Database, WriteOp};

use super::api_key;
use super::authorization::AuthorizationProvider;
use super::error::AuthError;
use super::jwt::JwtConfig;
use super::password;
use super::policy_engine::PolicyEngine;
use super::types::*;

const TOKEN_VERSION_FIELD: &str = "token_version";
const USER_IDENTITY_FIELD: &str = "user_identity";
const API_KEY_USER_IDENTITY_FIELD: &str = "user_identity";
const API_KEY_LOOKUP_FIELD: &str = "lookup_id";
const API_KEY_SECRET_HASH_FIELD: &str = "secret_hash";
const CREATED_AT_FIELD: &str = "created_at";
const UPDATED_AT_FIELD: &str = "updated_at";
const LEGACY_API_KEY_LOOKUP_COLLECTION: &str = "api_key_legacy_lookup";
const LEGACY_API_KEY_ENTRIES_FIELD: &str = "entries";
const AUTH_MIGRATION_PAGE_SIZE: usize = 100;

/// Central auth service managing users, API keys, policies, and token issuance.
pub struct AuthService {
    system_db: Arc<Database>,
    jwt_config: JwtConfig,
    policy_engine: PolicyEngine,
    user_state_lock: RwLock<()>,
}

impl AuthService {
    /// Initialize the auth service with the given system database and JWT settings.
    ///
    /// On first boot (no users exist), a root user is created with a random password.
    /// Returns `(service, optional_root_password)`.
    pub fn init(
        system_db: Arc<Database>,
        jwt_secret: &[u8],
        jwt_ttl_secs: u64,
    ) -> Result<(Self, Option<String>), String> {
        let jwt_config = JwtConfig::new(jwt_secret, jwt_ttl_secs);
        let policy_engine = PolicyEngine::new();

        let service = Self {
            system_db,
            jwt_config,
            policy_engine,
            user_state_lock: RwLock::new(()),
        };

        let root_password = service.bootstrap()?;
        service.repair_partial_api_key_migrations()?;
        service.rebuild_legacy_api_key_index()?;
        service.reload_policies()?;

        Ok((service, root_password))
    }

    /// Bootstrap the system: create a root user if no users exist.
    ///
    /// Returns `Some(password)` if a root user was created, `None` if users already exist.
    fn bootstrap(&self) -> Result<Option<String>, String> {
        let (docs, _) = self
            .system_db
            .list_documents("users", None, 1)
            .map_err(|e| format!("failed to check users: {e}"))?;

        if !docs.is_empty() {
            return Ok(None);
        }

        // Generate a random 24-character alphanumeric password
        let mut rng = rand::rng();
        let generated_password: String = (0..24)
            .map(|_| {
                let idx: u8 = rng.random_range(0..62);
                match idx {
                    0..10 => (b'0' + idx) as char,
                    10..36 => (b'a' + idx - 10) as char,
                    _ => (b'A' + idx - 36) as char,
                }
            })
            .collect();

        let hash =
            password::hash_password(&generated_password).map_err(|e| format!("bootstrap: {e}"))?;

        let mut fields = HashMap::new();
        fields.insert("username".to_string(), Value::String("root".to_string()));
        fields.insert("password_hash".to_string(), Value::String(hash));
        fields.insert("attributes".to_string(), Value::Object(HashMap::new()));
        fields.insert(
            USER_IDENTITY_FIELD.to_string(),
            Value::String(generate_user_identity()),
        );
        fields.insert(TOKEN_VERSION_FIELD.to_string(), Value::Int(0));
        let now = Utc::now().to_rfc3339();
        fields.insert(CREATED_AT_FIELD.to_string(), Value::String(now.clone()));
        fields.insert(UPDATED_AT_FIELD.to_string(), Value::String(now));

        let doc = Document {
            id: "users:root".to_string(),
            fields,
        };

        self.system_db
            .create_document(&doc)
            .map_err(|e| format!("failed to create root user: {e}"))?;

        Ok(Some(generated_password))
    }

    /// Authenticate with username and password, returning a JWT token on success.
    pub fn login(&self, username: &str, password_input: &str) -> Result<String, String> {
        let guard = self.user_state_lock.read();
        let doc = self.load_user_for_login(username)?;
        if doc.fields.contains_key(USER_IDENTITY_FIELD)
            && doc.fields.contains_key(TOKEN_VERSION_FIELD)
        {
            return self.issue_login_token(&doc, password_input);
        }

        drop(guard);
        let _guard = self.user_state_lock.write();
        let mut doc = self.load_user_for_login(username)?;
        let mut migrated = false;
        if !doc.fields.contains_key(USER_IDENTITY_FIELD) {
            doc.fields.insert(
                USER_IDENTITY_FIELD.to_string(),
                Value::String(generate_user_identity()),
            );
            migrated = true;
        }
        if !doc.fields.contains_key(TOKEN_VERSION_FIELD) {
            doc.fields
                .insert(TOKEN_VERSION_FIELD.to_string(), Value::Int(0));
            migrated = true;
        }
        if migrated {
            let updated = self
                .system_db
                .update_document(&doc)
                .map_err(|_| "invalid credentials".to_string())?;
            if !updated {
                return Err("invalid credentials".to_string());
            }
        }
        self.issue_login_token(&doc, password_input)
    }

    fn load_user_for_login(&self, username: &str) -> Result<Document, String> {
        self.system_db
            .get_document("users", username)
            .map_err(|_| "invalid credentials".to_string())?
            .ok_or_else(|| "invalid credentials".to_string())
    }

    fn issue_login_token(&self, doc: &Document, password_input: &str) -> Result<String, String> {
        let stored_hash = match doc.fields.get("password_hash") {
            Some(Value::String(hash)) => hash,
            _ => return Err("invalid credentials".to_string()),
        };
        if !password::verify_password(password_input, stored_hash) {
            return Err("invalid credentials".to_string());
        }

        let username = doc.key();
        let attributes = extract_attributes(doc);
        let user_identity =
            extract_user_identity(doc).map_err(|_| "invalid credentials".to_string())?;
        let token_version =
            extract_token_version(doc).map_err(|_| "invalid credentials".to_string())?;
        self.jwt_config
            .issue_versioned(username, &attributes, user_identity, token_version)
            .map_err(|_| "invalid credentials".to_string())
    }

    /// Authenticate a token and return the stable public ABAC subject.
    pub fn authenticate(&self, token: &str) -> Result<Subject, String> {
        self.authenticate_resolved(token)
            .map(ResolvedSubject::into_subject)
    }

    pub(crate) fn authenticate_resolved(&self, token: &str) -> Result<ResolvedSubject, String> {
        if api_key::is_api_key(token) {
            self.authenticate_api_key(token)
        } else {
            self.authenticate_jwt(token)
        }
    }

    /// Verify a JWT token and resolve its immutable server-side identity.
    fn authenticate_jwt(&self, token: &str) -> Result<ResolvedSubject, String> {
        let claims = self.jwt_config.verify(token)?;
        let claimed_identity = claims
            .user_identity
            .ok_or_else(|| "invalid JWT: legacy token has no user identity".to_string())?;
        let claimed_version = claims
            .token_version
            .ok_or_else(|| "invalid JWT: legacy token has no token version".to_string())?;
        let _guard = self.user_state_lock.read();
        let user_doc = self
            .system_db
            .get_document("users", &claims.sub)
            .map_err(|_| "invalid JWT: failed to load user".to_string())?
            .ok_or_else(|| "invalid JWT: user no longer exists".to_string())?;
        let current_identity = extract_user_identity(&user_doc)
            .map_err(|_| "invalid JWT: invalid user identity".to_string())?;
        let current_version = extract_token_version(&user_doc)
            .map_err(|_| "invalid JWT: invalid user token version".to_string())?;
        if claimed_identity != current_identity || claimed_version != current_version {
            return Err("invalid JWT: token has been revoked".to_string());
        }

        Ok(ResolvedSubject::new(
            Subject {
                user_id: claims.sub,
                attributes: extract_attributes(&user_doc),
            },
            current_identity.to_string(),
        ))
    }

    /// Authenticate current keys with a direct random lookup ID and legacy keys through
    /// the startup-built hash index. No request performs a metadata collection scan.
    fn authenticate_api_key(&self, key: &str) -> Result<ResolvedSubject, String> {
        match api_key::parse_api_key(key)
            .ok_or_else(|| "invalid API key: malformed key".to_string())?
        {
            api_key::ParsedApiKey::Direct { lookup_id, secret } => {
                let _guard = self.user_state_lock.read();
                self.authenticate_api_key_lookup(&lookup_id, |stored_hash| {
                    api_key::verify_secret(secret, stored_hash)
                })?
                .ok_or_else(|| "invalid API key".to_string())
            }
            api_key::ParsedApiKey::Legacy => self.authenticate_legacy_api_key(key),
        }
    }

    fn authenticate_api_key_lookup(
        &self,
        lookup_id: &str,
        verify_secret: impl FnOnce(&str) -> bool,
    ) -> Result<Option<ResolvedSubject>, String> {
        let Some(lookup_doc) = self
            .system_db
            .get_document("api_key_lookup", lookup_id)
            .map_err(|e| format!("failed to lookup API key: {e}"))?
        else {
            return Ok(None);
        };
        let stored_hash = match lookup_doc.fields.get(API_KEY_SECRET_HASH_FIELD) {
            Some(Value::String(hash)) => hash,
            _ => return Err("invalid API key: missing secret hash".to_string()),
        };
        if !verify_secret(stored_hash) {
            return Err("invalid API key".to_string());
        }

        let entry = LegacyApiKeyEntry::from_lookup_document(&lookup_doc)?;
        let metadata = self
            .system_db
            .get_document("api_keys", &entry.name)
            .map_err(|e| format!("failed to load API key metadata: {e}"))?
            .ok_or_else(|| "invalid API key: metadata not found".to_string())?;
        if string_field(
            &metadata,
            API_KEY_LOOKUP_FIELD,
            "invalid API key: metadata has no lookup ID",
        )? != lookup_id
            || !entry.matches_metadata(&metadata)?
        {
            return Err("invalid API key: metadata mismatch".to_string());
        }

        self.resolve_api_key_subject(&entry.username, &entry.user_identity)
            .map(Some)
    }

    fn authenticate_legacy_api_key(&self, key: &str) -> Result<ResolvedSubject, String> {
        let legacy_hash = api_key::hash_legacy_key(key);
        let lookup_id = api_key::legacy_lookup_id(key);
        let _guard = self.user_state_lock.write();

        if let Some(subject) =
            self.repair_or_authenticate_migrated_legacy(key, &legacy_hash, &lookup_id)?
        {
            return Ok(subject);
        }

        let index = self
            .system_db
            .get_document(LEGACY_API_KEY_LOOKUP_COLLECTION, &legacy_hash)
            .map_err(|e| format!("failed to lookup legacy API key index: {e}"))?
            .ok_or_else(|| "invalid API key".to_string())?;
        for entry in legacy_index_entries(&index)? {
            let Some(metadata) = self
                .system_db
                .get_document("api_keys", &entry.name)
                .map_err(|e| format!("failed to load legacy API key metadata: {e}"))?
            else {
                continue;
            };
            let Some(Value::String(stored_hash)) = metadata.fields.get("key_hash") else {
                continue;
            };
            if !api_key::verify_legacy_lookup_hash(key, stored_hash)
                || !entry.matches_metadata(&metadata)?
            {
                continue;
            }
            self.resolve_api_key_subject(&entry.username, &entry.user_identity)?;
            self.finalize_legacy_api_key_migration(
                key,
                &legacy_hash,
                &lookup_id,
                metadata,
                &entry,
            )?;
            return self.resolve_api_key_subject(&entry.username, &entry.user_identity);
        }

        Err("invalid API key".to_string())
    }

    fn repair_or_authenticate_migrated_legacy(
        &self,
        key: &str,
        legacy_hash: &str,
        lookup_id: &str,
    ) -> Result<Option<ResolvedSubject>, String> {
        let Some(lookup_doc) = self
            .system_db
            .get_document("api_key_lookup", lookup_id)
            .map_err(|e| format!("failed to lookup migrated API key: {e}"))?
        else {
            return Ok(None);
        };
        let stored_hash = string_field(
            &lookup_doc,
            API_KEY_SECRET_HASH_FIELD,
            "invalid API key: missing secret hash",
        )?;
        if !api_key::verify_legacy_secret(key, stored_hash) {
            return Err("invalid API key: lookup collision during migration".to_string());
        }
        let entry = LegacyApiKeyEntry::from_lookup_document(&lookup_doc)?;
        let metadata = self
            .system_db
            .get_document("api_keys", &entry.name)
            .map_err(|e| format!("failed to load migrated API key metadata: {e}"))?
            .ok_or_else(|| "invalid API key: metadata not found".to_string())?;

        if metadata.fields.get(API_KEY_LOOKUP_FIELD) == Some(&Value::String(lookup_id.to_string()))
            && entry.matches_metadata(&metadata)?
        {
            return self
                .resolve_api_key_subject(&entry.username, &entry.user_identity)
                .map(Some);
        }

        let Some(Value::String(stored_legacy_hash)) = metadata.fields.get("key_hash") else {
            return Err("invalid API key: metadata mismatch".to_string());
        };
        if stored_legacy_hash != legacy_hash || !entry.matches_metadata(&metadata)? {
            return Err("invalid API key: metadata mismatch".to_string());
        }
        self.finalize_legacy_api_key_migration(key, legacy_hash, lookup_id, metadata, &entry)?;
        self.resolve_api_key_subject(&entry.username, &entry.user_identity)
            .map(Some)
    }

    fn finalize_legacy_api_key_migration(
        &self,
        key: &str,
        legacy_hash: &str,
        lookup_id: &str,
        mut metadata: Document,
        entry: &LegacyApiKeyEntry,
    ) -> Result<(), String> {
        let lookup_doc = api_key_lookup_document(
            lookup_id,
            &entry.name,
            &entry.username,
            &entry.user_identity,
            api_key::hash_legacy_secret(key),
        );
        if let Some(existing) = self
            .system_db
            .get_document("api_key_lookup", lookup_id)
            .map_err(|e| format!("failed to check migrated API key lookup: {e}"))?
        {
            let existing_entry = LegacyApiKeyEntry::from_lookup_document(&existing)?;
            let existing_hash = string_field(
                &existing,
                API_KEY_SECRET_HASH_FIELD,
                "invalid API key: missing secret hash",
            )?;
            if &existing_entry != entry || !api_key::verify_legacy_secret(key, existing_hash) {
                return Err("invalid API key: lookup collision during migration".to_string());
            }
        }

        metadata.fields.remove("key_hash");
        metadata.fields.insert(
            API_KEY_LOOKUP_FIELD.to_string(),
            Value::String(lookup_id.to_string()),
        );
        let mut operations = vec![WriteOp::InsertDoc(metadata), WriteOp::InsertDoc(lookup_doc)];
        self.push_legacy_index_removal(&mut operations, legacy_hash, &entry.name)?;
        self.system_db
            .atomic_write_batch(operations)
            .map_err(|e| format!("failed to atomically migrate legacy API key: {e}"))
    }

    fn resolve_api_key_subject(
        &self,
        username: &str,
        key_identity: &str,
    ) -> Result<ResolvedSubject, String> {
        let user_doc = self
            .system_db
            .get_document("users", username)
            .map_err(|e| format!("failed to load user for API key: {e}"))?
            .ok_or_else(|| "invalid API key: associated user not found".to_string())?;
        let principal_identity = extract_user_identity(&user_doc)
            .map_err(|_| "invalid API key: invalid user identity".to_string())?;
        if principal_identity != key_identity {
            return Err("invalid API key: user identity mismatch".to_string());
        }
        Ok(ResolvedSubject::new(
            Subject {
                user_id: username.to_string(),
                attributes: extract_attributes(&user_doc),
            },
            principal_identity.to_string(),
        ))
    }

    /// Repair the lookup-first state left by older non-atomic migrations. The lookup
    /// document already contains the hashed secret, so startup can finish metadata
    /// finalization without seeing or scanning for the plaintext key.
    fn repair_partial_api_key_migrations(&self) -> Result<(), String> {
        let mut after: Option<String> = None;
        loop {
            let (lookups, cursor) = self
                .system_db
                .list_documents("api_key_lookup", after.as_deref(), AUTH_MIGRATION_PAGE_SIZE)
                .map_err(|error| format!("failed to list API key lookups for repair: {error}"))?;
            for lookup in lookups {
                let lookup_id = lookup.key().to_string();
                let Ok(entry) = LegacyApiKeyEntry::from_lookup_document(&lookup) else {
                    continue;
                };
                if !matches!(
                    lookup.fields.get(API_KEY_SECRET_HASH_FIELD),
                    Some(Value::String(hash)) if !hash.is_empty()
                ) {
                    continue;
                }
                let Some(mut metadata) = self
                    .system_db
                    .get_document("api_keys", &entry.name)
                    .map_err(|error| {
                        format!("failed to load API key metadata for startup repair: {error}")
                    })?
                else {
                    continue;
                };
                match metadata.fields.get(API_KEY_LOOKUP_FIELD) {
                    Some(Value::String(existing)) if existing == &lookup_id => continue,
                    Some(Value::String(existing)) => {
                        return Err(format!(
                            "API key lookup collision during startup repair: metadata references '{existing}', lookup is '{lookup_id}'"
                        ));
                    }
                    Some(_) => {
                        return Err(
                            "invalid API key metadata lookup ID during startup repair".to_string()
                        );
                    }
                    None => {}
                }
                let Some(Value::String(legacy_hash)) = metadata.fields.get("key_hash").cloned()
                else {
                    continue;
                };
                if !entry.matches_metadata(&metadata)? {
                    continue;
                }
                metadata.fields.remove("key_hash");
                metadata
                    .fields
                    .insert(API_KEY_LOOKUP_FIELD.to_string(), Value::String(lookup_id));
                let mut operations = vec![WriteOp::InsertDoc(metadata)];
                self.push_legacy_index_removal(&mut operations, &legacy_hash, &entry.name)?;
                self.system_db
                    .atomic_write_batch(operations)
                    .map_err(|error| {
                        format!("failed to atomically repair partial API key migration: {error}")
                    })?;
            }
            match cursor {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        Ok(())
    }

    fn rebuild_legacy_api_key_index(&self) -> Result<(), String> {
        let mut after: Option<String> = None;
        loop {
            let (documents, cursor) = self
                .system_db
                .list_documents("api_keys", after.as_deref(), AUTH_MIGRATION_PAGE_SIZE)
                .map_err(|e| format!("failed to list legacy API keys: {e}"))?;
            for metadata in documents {
                let Some(Value::String(legacy_hash)) = metadata.fields.get("key_hash") else {
                    continue;
                };
                let Some(entry) = LegacyApiKeyEntry::from_legacy_metadata(&metadata) else {
                    // Legacy records without immutable identity deliberately remain unindexed.
                    continue;
                };
                self.upsert_legacy_index_entry(legacy_hash, entry)?;
            }
            match cursor {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        Ok(())
    }

    fn upsert_legacy_index_entry(
        &self,
        legacy_hash: &str,
        entry: LegacyApiKeyEntry,
    ) -> Result<(), String> {
        let mut entries = match self
            .system_db
            .get_document(LEGACY_API_KEY_LOOKUP_COLLECTION, legacy_hash)
            .map_err(|e| format!("failed to load legacy API key index: {e}"))?
        {
            Some(document) => legacy_index_entries(&document)?,
            None => Vec::new(),
        };
        if let Some(existing) = entries.iter_mut().find(|item| item.name == entry.name) {
            *existing = entry;
        } else {
            entries.push(entry);
        }
        self.system_db
            .atomic_write_batch(vec![WriteOp::InsertDoc(legacy_index_document(
                legacy_hash,
                &entries,
            ))])
            .map_err(|e| format!("failed to persist legacy API key index: {e}"))
    }

    fn push_legacy_index_removal(
        &self,
        operations: &mut Vec<WriteOp>,
        legacy_hash: &str,
        name: &str,
    ) -> Result<(), String> {
        let Some(document) = self
            .system_db
            .get_document(LEGACY_API_KEY_LOOKUP_COLLECTION, legacy_hash)
            .map_err(|e| format!("failed to load legacy API key index: {e}"))?
        else {
            return Ok(());
        };
        let mut entries = legacy_index_entries(&document)?;
        entries.retain(|entry| entry.name != name);
        if entries.is_empty() {
            operations.push(WriteOp::DeleteDoc {
                collection: LEGACY_API_KEY_LOOKUP_COLLECTION.to_string(),
                key: legacy_hash.to_string(),
            });
        } else {
            operations.push(WriteOp::InsertDoc(legacy_index_document(
                legacy_hash,
                &entries,
            )));
        }
        Ok(())
    }

    /// Evaluate an authorization decision for the given subject, action, and resource.
    pub fn authorize(
        &self,
        subject: &Subject,
        action: Action,
        resource: &Resource,
    ) -> AuthzDecision {
        self.policy_engine.evaluate(subject, action, resource)
    }

    /// Create a new user with the given username and password.
    pub fn create_user(&self, username: &str, password_input: &str) -> Result<(), String> {
        self.create_user_typed(username, password_input)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn create_user_typed(
        &self,
        username: &str,
        password_input: &str,
    ) -> Result<(), AuthError> {
        self.create_user_with_attributes_typed(username, password_input, HashMap::new())
    }

    pub(crate) fn create_user_with_attributes_typed(
        &self,
        username: &str,
        password_input: &str,
        attributes: HashMap<String, String>,
    ) -> Result<(), AuthError> {
        let hash = password::hash_password(password_input)
            .map_err(|error| AuthError::internal(format!("create_user: {error}")))?;
        let _guard = self.user_state_lock.write();
        let now = Utc::now().to_rfc3339();
        let doc = Document {
            id: format!("users:{username}"),
            fields: HashMap::from([
                ("username".to_string(), Value::String(username.to_string())),
                ("password_hash".to_string(), Value::String(hash)),
                (
                    "attributes".to_string(),
                    Value::Object(
                        attributes
                            .into_iter()
                            .map(|(key, value)| (key, Value::String(value)))
                            .collect(),
                    ),
                ),
                (
                    USER_IDENTITY_FIELD.to_string(),
                    Value::String(generate_user_identity()),
                ),
                (TOKEN_VERSION_FIELD.to_string(), Value::Int(0)),
                (CREATED_AT_FIELD.to_string(), Value::String(now.clone())),
                (UPDATED_AT_FIELD.to_string(), Value::String(now)),
            ]),
        };
        let created = self
            .system_db
            .create_document(&doc)
            .map_err(|error| AuthError::internal(format!("failed to create user: {error}")))?;
        if !created {
            return Err(AuthError::already_exists("user", username));
        }
        Ok(())
    }

    /// Drop a user by username. Returns `Ok(true)` if deleted, `Ok(false)` if not found.
    ///
    /// The root user cannot be dropped. Associated API keys are cascade-deleted.
    pub fn drop_user(&self, username: &str) -> Result<bool, String> {
        compatibility_drop(self.drop_user_typed(username))
    }

    pub(crate) fn drop_user_typed(&self, username: &str) -> Result<(), AuthError> {
        if username == "root" {
            return Err(AuthError::invalid("cannot drop the root user"));
        }

        let _guard = self.user_state_lock.write();
        let deleted = self
            .system_db
            .delete_document("users", username)
            .map_err(|error| AuthError::internal(format!("failed to drop user: {error}")))?;
        if !deleted {
            return Err(AuthError::not_found("user", username));
        }
        self.delete_api_keys_for_user(username)?;
        Ok(())
    }

    /// Set (upsert) attributes on a user.
    pub fn alter_user_set(
        &self,
        username: &str,
        attrs: HashMap<String, String>,
    ) -> Result<(), String> {
        self.alter_user_set_typed(username, attrs)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn alter_user_set_typed(
        &self,
        username: &str,
        attrs: HashMap<String, String>,
    ) -> Result<(), AuthError> {
        self.alter_user_attributes_typed(username, attrs, &[])
    }

    pub(crate) fn alter_user_attributes_typed(
        &self,
        username: &str,
        set: HashMap<String, String>,
        remove: &[String],
    ) -> Result<(), AuthError> {
        let _guard = self.user_state_lock.write();
        let mut doc = self.load_user_for_management(username)?;
        let attributes = match doc.fields.get_mut("attributes") {
            Some(Value::Object(obj)) => obj,
            _ => {
                doc.fields
                    .insert("attributes".to_string(), Value::Object(HashMap::new()));
                match doc.fields.get_mut("attributes") {
                    Some(Value::Object(obj)) => obj,
                    _ => unreachable!(),
                }
            }
        };
        for (key, value) in set {
            attributes.insert(key, Value::String(value));
        }
        for key in remove {
            attributes.remove(key);
        }
        self.persist_managed_user(&mut doc)
    }

    /// Remove attribute keys from a user.
    pub fn alter_user_remove(&self, username: &str, attr_keys: &[String]) -> Result<(), String> {
        self.alter_user_remove_typed(username, attr_keys)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn alter_user_remove_typed(
        &self,
        username: &str,
        attr_keys: &[String],
    ) -> Result<(), AuthError> {
        let _guard = self.user_state_lock.write();
        let mut doc = self.load_user_for_management(username)?;
        if let Some(Value::Object(attributes)) = doc.fields.get_mut("attributes") {
            for key in attr_keys {
                attributes.remove(key);
            }
        }
        self.persist_managed_user(&mut doc)
    }

    /// Change a user's password.
    pub fn alter_user_password(&self, username: &str, new_password: &str) -> Result<(), String> {
        self.alter_user_password_typed(username, new_password)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn alter_user_password_typed(
        &self,
        username: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        let hash = password::hash_password(new_password)
            .map_err(|error| AuthError::internal(format!("alter_user_password: {error}")))?;
        let _guard = self.user_state_lock.write();
        let mut doc = self.load_user_for_management(username)?;
        doc.fields
            .insert("password_hash".to_string(), Value::String(hash));
        self.persist_managed_user(&mut doc)
    }

    fn load_user_for_management(&self, username: &str) -> Result<Document, AuthError> {
        self.system_db
            .get_document("users", username)
            .map_err(|error| AuthError::internal(format!("failed to get user: {error}")))?
            .ok_or_else(|| AuthError::not_found("user", username))
    }

    fn persist_managed_user(&self, doc: &mut Document) -> Result<(), AuthError> {
        ensure_user_identity(doc).map_err(AuthError::internal)?;
        bump_token_version(doc).map_err(AuthError::internal)?;
        doc.fields.insert(
            UPDATED_AT_FIELD.to_string(),
            Value::String(Utc::now().to_rfc3339()),
        );
        update_user_document(&self.system_db, doc).map_err(AuthError::internal)
    }

    pub(crate) fn list_users_typed(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ManagedUser>, Option<String>), AuthError> {
        let _guard = self.user_state_lock.read();
        let key_counts = self.api_key_counts_locked()?;
        let (documents, next_cursor) = self
            .system_db
            .list_documents("users", after, limit.clamp(1, 100))
            .map_err(|error| AuthError::internal(format!("failed to list users: {error}")))?;
        let users = documents
            .into_iter()
            .map(|document| managed_user_from_document(&document, &key_counts))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((users, next_cursor))
    }

    pub(crate) fn get_user_typed(&self, username: &str) -> Result<ManagedUser, AuthError> {
        let _guard = self.user_state_lock.read();
        let document = self
            .system_db
            .get_document("users", username)
            .map_err(|error| AuthError::internal(format!("failed to get user: {error}")))?
            .ok_or_else(|| AuthError::not_found("user", username))?;
        let key_counts = self.api_key_counts_locked()?;
        managed_user_from_document(&document, &key_counts)
    }

    fn api_key_counts_locked(&self) -> Result<HashMap<String, usize>, AuthError> {
        let mut counts = HashMap::new();
        let mut after: Option<String> = None;
        loop {
            let (documents, next_cursor) = self
                .system_db
                .list_documents("api_keys", after.as_deref(), 100)
                .map_err(|error| {
                    AuthError::internal(format!("failed to count API keys: {error}"))
                })?;
            for document in documents {
                if let Some(Value::String(username)) = document.fields.get("username") {
                    *counts.entry(username.clone()).or_insert(0) += 1;
                }
            }
            let Some(cursor) = next_cursor else {
                break;
            };
            after = Some(cursor);
        }
        Ok(counts)
    }

    /// Create a new API key for the given user. Returns the plaintext key once.
    pub fn create_api_key(&self, name: &str, username: &str) -> Result<String, String> {
        self.create_api_key_typed(name, username)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn create_api_key_typed(
        &self,
        name: &str,
        username: &str,
    ) -> Result<String, AuthError> {
        self.create_api_key_with_plaintext_typed(name, username, api_key::generate_api_key())
    }

    fn create_api_key_with_plaintext_typed(
        &self,
        name: &str,
        username: &str,
        plaintext: String,
    ) -> Result<String, AuthError> {
        let _guard = self.user_state_lock.write();
        let user_doc = self
            .system_db
            .get_document("users", username)
            .map_err(|error| AuthError::internal(format!("failed to verify user: {error}")))?
            .ok_or_else(|| AuthError::not_found("user", username))?;
        let user_identity = extract_user_identity(&user_doc)
            .map_err(|_| AuthError::invalid(format!("user '{username}' has invalid identity")))?;
        let api_key::ParsedApiKey::Direct { lookup_id, secret } =
            api_key::parse_api_key(&plaintext)
                .ok_or_else(|| AuthError::invalid("generated API key is malformed"))?
        else {
            return Err(AuthError::invalid(
                "generated API key does not use the direct format",
            ));
        };
        let secret_hash = api_key::hash_secret(secret)
            .ok_or_else(|| AuthError::invalid("generated API key secret is malformed"))?;
        let metadata = Document {
            id: format!("api_keys:{name}"),
            fields: HashMap::from([
                ("name".to_string(), Value::String(name.to_string())),
                ("username".to_string(), Value::String(username.to_string())),
                (
                    API_KEY_LOOKUP_FIELD.to_string(),
                    Value::String(lookup_id.clone()),
                ),
                (
                    API_KEY_USER_IDENTITY_FIELD.to_string(),
                    Value::String(user_identity.to_string()),
                ),
                (
                    CREATED_AT_FIELD.to_string(),
                    Value::String(Utc::now().to_rfc3339()),
                ),
            ]),
        };
        let lookup_doc =
            api_key_lookup_document(&lookup_id, name, username, user_identity, secret_hash);
        let metadata_id = metadata.id.clone();
        let lookup_id_full = lookup_doc.id.clone();
        match self.system_db.atomic_write_batch(vec![
            WriteOp::CreateDoc(metadata),
            WriteOp::CreateDoc(lookup_doc),
        ]) {
            Ok(()) => Ok(plaintext),
            Err(StorageError::AlreadyExists(id)) if id == metadata_id => {
                Err(AuthError::already_exists("API key", name))
            }
            Err(StorageError::AlreadyExists(id)) if id == lookup_id_full => {
                Err(AuthError::internal(format!(
                    "failed to create API key: random lookup ID collision '{lookup_id}'"
                )))
            }
            Err(error) => Err(AuthError::internal(format!(
                "failed to atomically create API key: {error}"
            ))),
        }
    }

    /// Drop an API key by name and atomically remove its authentication lookup state.
    pub fn drop_api_key(&self, name: &str) -> Result<bool, String> {
        compatibility_drop(self.drop_api_key_typed(name))
    }

    pub(crate) fn drop_api_key_typed(&self, name: &str) -> Result<(), AuthError> {
        let _guard = self.user_state_lock.write();
        self.drop_api_key_locked(name)
    }

    pub(crate) fn list_api_keys_typed(
        &self,
        after: Option<&str>,
        limit: usize,
        username: Option<&str>,
    ) -> Result<(Vec<ManagedApiKey>, Option<String>), AuthError> {
        let _guard = self.user_state_lock.read();
        let limit = limit.clamp(1, 100);
        let mut cursor = after.map(str::to_string);
        let mut result = Vec::new();

        loop {
            let (documents, next_cursor) = self
                .system_db
                .list_documents("api_keys", cursor.as_deref(), limit)
                .map_err(|error| {
                    AuthError::internal(format!("failed to list API keys: {error}"))
                })?;
            for document in documents {
                let item = managed_api_key_from_document(&document)?;
                let matches = username.is_none_or(|filter| item.username == filter);
                if matches {
                    result.push(item);
                }
                if result.len() == limit {
                    return Ok((result, Some(document.key().to_string())));
                }
            }
            let Some(next) = next_cursor else {
                return Ok((result, None));
            };
            cursor = Some(next);
        }
    }

    pub(crate) fn get_api_key_typed(&self, name: &str) -> Result<ManagedApiKey, AuthError> {
        let _guard = self.user_state_lock.read();
        let document = self
            .system_db
            .get_document("api_keys", name)
            .map_err(|error| AuthError::internal(format!("failed to get API key: {error}")))?
            .ok_or_else(|| AuthError::not_found("API key", name))?;
        managed_api_key_from_document(&document)
    }

    pub(crate) fn api_key_name_for_token(&self, token: &str) -> Result<Option<String>, AuthError> {
        let lookup_id = match api_key::parse_api_key(token) {
            Some(api_key::ParsedApiKey::Direct { lookup_id, .. }) => lookup_id,
            Some(api_key::ParsedApiKey::Legacy) => api_key::legacy_lookup_id(token),
            None => return Ok(None),
        };
        let _guard = self.user_state_lock.read();
        let document = self
            .system_db
            .get_document("api_key_lookup", &lookup_id)
            .map_err(|error| AuthError::internal(format!("failed to identify API key: {error}")))?;
        document
            .map(|document| {
                string_field(&document, "name", "API key lookup has no name")
                    .map(str::to_string)
                    .map_err(AuthError::internal)
            })
            .transpose()
    }

    fn drop_api_key_locked(&self, name: &str) -> Result<(), AuthError> {
        let metadata = self
            .system_db
            .get_document("api_keys", name)
            .map_err(|error| AuthError::internal(format!("failed to load API key: {error}")))?
            .ok_or_else(|| AuthError::not_found("API key", name))?;
        let mut operations = vec![WriteOp::DeleteDoc {
            collection: "api_keys".to_string(),
            key: name.to_string(),
        }];
        if let Some(Value::String(lookup_id)) = metadata.fields.get(API_KEY_LOOKUP_FIELD) {
            operations.push(WriteOp::DeleteDoc {
                collection: "api_key_lookup".to_string(),
                key: lookup_id.clone(),
            });
        }
        if let Some(Value::String(legacy_hash)) = metadata.fields.get("key_hash") {
            self.push_legacy_index_removal(&mut operations, legacy_hash, name)
                .map_err(AuthError::internal)?;
        }
        self.system_db
            .atomic_write_batch(operations)
            .map_err(|error| {
                AuthError::internal(format!("failed to atomically drop API key: {error}"))
            })
    }

    /// Store a policy to the database and add it to the in-memory engine.
    pub fn create_policy(&self, policy: &Policy) -> Result<(), String> {
        self.create_policy_typed(policy)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn create_policy_typed(&self, policy: &Policy) -> Result<(), AuthError> {
        let doc = policy_to_document(policy);
        let created = self
            .system_db
            .create_document(&doc)
            .map_err(|error| AuthError::internal(format!("failed to create policy: {error}")))?;
        if !created {
            return Err(AuthError::already_exists("policy", &policy.name));
        }
        self.policy_engine.add_policy(policy.clone());
        Ok(())
    }

    /// Remove a policy from the database and the in-memory engine.
    /// Returns `Ok(true)` if deleted, `Ok(false)` if not found.
    pub fn drop_policy(&self, name: &str) -> Result<bool, String> {
        compatibility_drop(self.drop_policy_typed(name))
    }

    pub(crate) fn drop_policy_typed(&self, name: &str) -> Result<(), AuthError> {
        let deleted = self
            .system_db
            .delete_document("policies", name)
            .map_err(|error| AuthError::internal(format!("failed to drop policy: {error}")))?;
        if !deleted {
            return Err(AuthError::not_found("policy", name));
        }
        self.policy_engine.remove_policy(name);
        Ok(())
    }

    /// Reload all policies from the database into the in-memory engine.
    pub fn reload_policies(&self) -> Result<(), String> {
        let mut policies = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let (docs, cursor) = self
                .system_db
                .list_documents("policies", after.as_deref(), 100)
                .map_err(|e| format!("failed to load policies: {e}"))?;

            for doc in &docs {
                match document_to_policy(doc) {
                    Ok(policy) => policies.push(policy),
                    Err(e) => {
                        eprintln!("warning: skipping malformed policy '{}': {e}", doc.id);
                    }
                }
            }

            match cursor {
                Some(c) => after = Some(c),
                None => break,
            }
        }

        self.policy_engine.load_policies(policies);
        Ok(())
    }

    // ─── Collection Attributes ───

    /// Get resource attributes for a collection (used during authorization).
    ///
    /// Collection attributes are stored as documents in the `collection_attrs`
    /// collection within the `_system` database, keyed by `{database}:{collection}`.
    pub fn get_collection_attributes(
        &self,
        database: &str,
        collection: &str,
    ) -> HashMap<String, String> {
        let key = format!("{database}:{collection}");
        match self.system_db.get_document("collection_attrs", &key) {
            Ok(Some(doc)) => {
                let mut attrs = HashMap::new();
                for (k, v) in &doc.fields {
                    if let Value::String(s) = v {
                        attrs.insert(k.clone(), s.clone());
                    }
                }
                attrs
            }
            _ => HashMap::new(),
        }
    }

    /// Set (upsert) attributes on a collection resource.
    ///
    /// Merges the given attributes into any existing ones. Creates the document
    /// if it does not already exist.
    pub fn set_collection_attributes(
        &self,
        database: &str,
        collection: &str,
        attrs: HashMap<String, String>,
    ) -> Result<(), String> {
        let key = format!("{database}:{collection}");
        let id = format!("collection_attrs:{key}");

        // Get existing or create new
        let mut doc = self
            .system_db
            .get_document("collection_attrs", &key)
            .unwrap_or(None)
            .unwrap_or_else(|| Document {
                id: id.clone(),
                fields: HashMap::new(),
            });

        for (k, v) in attrs {
            doc.fields.insert(k, Value::String(v));
        }

        // Try update first, if not found then create
        let updated = self
            .system_db
            .update_document(&doc)
            .map_err(|e| format!("failed to set collection attributes: {e}"))?;
        if !updated {
            self.system_db
                .create_document(&doc)
                .map_err(|e| format!("failed to create collection attributes: {e}"))?;
        }
        Ok(())
    }

    /// Remove specific attribute keys from a collection resource.
    ///
    /// If the collection has no stored attributes, this is a no-op.
    pub fn remove_collection_attributes(
        &self,
        database: &str,
        collection: &str,
        keys: &[String],
    ) -> Result<(), String> {
        let key = format!("{database}:{collection}");
        let mut doc = match self.system_db.get_document("collection_attrs", &key) {
            Ok(Some(doc)) => doc,
            _ => return Ok(()), // Nothing to remove
        };

        for k in keys {
            doc.fields.remove(k);
        }

        self.system_db
            .update_document(&doc)
            .map_err(|e| format!("failed to update collection attributes: {e}"))?;
        Ok(())
    }

    /// Returns a reference to the policy engine.
    pub fn policy_engine(&self) -> &PolicyEngine {
        &self.policy_engine
    }

    /// Returns a reference to the JWT configuration.
    pub fn jwt_config(&self) -> &JwtConfig {
        &self.jwt_config
    }

    /// Delete all API keys belonging to a given user (used during cascade user deletion).
    fn delete_api_keys_for_user(&self, username: &str) -> Result<(), AuthError> {
        let mut after: Option<String> = None;
        let mut to_delete = Vec::new();

        loop {
            let (docs, cursor) = self
                .system_db
                .list_documents("api_keys", after.as_deref(), 100)
                .map_err(|error| {
                    AuthError::internal(format!("failed to list API keys: {error}"))
                })?;

            for doc in &docs {
                if let Some(Value::String(owner)) = doc.fields.get("username")
                    && owner == username
                {
                    to_delete.push(doc.key().to_string());
                }
            }

            match cursor {
                Some(c) => after = Some(c),
                None => break,
            }
        }

        for key_name in to_delete {
            self.drop_api_key_locked(&key_name)?;
        }

        Ok(())
    }
}

fn compatibility_drop(result: Result<(), AuthError>) -> Result<bool, String> {
    match result {
        Ok(()) => Ok(true),
        Err(AuthError::NotFound { .. }) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyApiKeyEntry {
    name: String,
    username: String,
    user_identity: String,
}

impl LegacyApiKeyEntry {
    fn from_legacy_metadata(metadata: &Document) -> Option<Self> {
        let Value::String(name) = metadata.fields.get("name")? else {
            return None;
        };
        let Value::String(username) = metadata.fields.get("username")? else {
            return None;
        };
        let Value::String(user_identity) = metadata.fields.get(API_KEY_USER_IDENTITY_FIELD)? else {
            return None;
        };
        if name.is_empty() || username.is_empty() || user_identity.is_empty() {
            return None;
        }
        Some(Self {
            name: name.clone(),
            username: username.clone(),
            user_identity: user_identity.clone(),
        })
    }

    fn from_lookup_document(document: &Document) -> Result<Self, String> {
        Ok(Self {
            name: string_field(document, "name", "invalid API key: missing name")?.to_string(),
            username: string_field(document, "username", "invalid API key: missing username")?
                .to_string(),
            user_identity: string_field(
                document,
                API_KEY_USER_IDENTITY_FIELD,
                "invalid API key: missing user identity",
            )?
            .to_string(),
        })
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let Value::Object(fields) = value else {
            return Err("invalid legacy API key index entry".to_string());
        };
        let string = |field: &str| match fields.get(field) {
            Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
            _ => Err(format!(
                "invalid legacy API key index entry: missing {field}"
            )),
        };
        Ok(Self {
            name: string("name")?,
            username: string("username")?,
            user_identity: string(API_KEY_USER_IDENTITY_FIELD)?,
        })
    }

    fn to_value(&self) -> Value {
        Value::Object(HashMap::from([
            ("name".to_string(), Value::String(self.name.clone())),
            ("username".to_string(), Value::String(self.username.clone())),
            (
                API_KEY_USER_IDENTITY_FIELD.to_string(),
                Value::String(self.user_identity.clone()),
            ),
        ]))
    }

    fn matches_metadata(&self, metadata: &Document) -> Result<bool, String> {
        Ok(
            string_field(metadata, "name", "invalid API key: missing name")? == self.name
                && string_field(metadata, "username", "invalid API key: missing username")?
                    == self.username
                && string_field(
                    metadata,
                    API_KEY_USER_IDENTITY_FIELD,
                    "invalid API key: missing user identity",
                )? == self.user_identity,
        )
    }
}

fn legacy_index_entries(document: &Document) -> Result<Vec<LegacyApiKeyEntry>, String> {
    let Some(Value::Array(entries)) = document.fields.get(LEGACY_API_KEY_ENTRIES_FIELD) else {
        return Err("invalid legacy API key index: missing entries".to_string());
    };
    entries.iter().map(LegacyApiKeyEntry::from_value).collect()
}

fn legacy_index_document(legacy_hash: &str, entries: &[LegacyApiKeyEntry]) -> Document {
    Document {
        id: format!("{LEGACY_API_KEY_LOOKUP_COLLECTION}:{legacy_hash}"),
        fields: HashMap::from([(
            LEGACY_API_KEY_ENTRIES_FIELD.to_string(),
            Value::Array(entries.iter().map(LegacyApiKeyEntry::to_value).collect()),
        )]),
    }
}

fn api_key_lookup_document(
    lookup_id: &str,
    name: &str,
    username: &str,
    user_identity: &str,
    secret_hash: String,
) -> Document {
    Document {
        id: format!("api_key_lookup:{lookup_id}"),
        fields: HashMap::from([
            ("name".to_string(), Value::String(name.to_string())),
            ("username".to_string(), Value::String(username.to_string())),
            (
                API_KEY_SECRET_HASH_FIELD.to_string(),
                Value::String(secret_hash),
            ),
            (
                API_KEY_USER_IDENTITY_FIELD.to_string(),
                Value::String(user_identity.to_string()),
            ),
        ]),
    }
}

fn string_field<'a>(doc: &'a Document, field: &str, error: &str) -> Result<&'a str, String> {
    match doc.fields.get(field) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(error.to_string()),
    }
}

fn generate_user_identity() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

fn extract_user_identity(doc: &Document) -> Result<&str, String> {
    match doc.fields.get(USER_IDENTITY_FIELD) {
        Some(Value::String(identity)) if !identity.is_empty() => Ok(identity),
        Some(Value::String(_)) => Err("user_identity must not be empty".to_string()),
        Some(_) => Err("user_identity must be a string".to_string()),
        None => Err("user_identity is missing".to_string()),
    }
}

fn ensure_user_identity(doc: &mut Document) -> Result<&str, String> {
    if !doc.fields.contains_key(USER_IDENTITY_FIELD) {
        doc.fields.insert(
            USER_IDENTITY_FIELD.to_string(),
            Value::String(generate_user_identity()),
        );
    }
    extract_user_identity(doc)
}

fn update_user_document(system_db: &Database, doc: &Document) -> Result<(), String> {
    let updated = system_db
        .update_document(doc)
        .map_err(|e| format!("failed to update user: {e}"))?;
    if !updated {
        return Err(format!("user '{}' changed concurrently", doc.key()));
    }
    Ok(())
}

fn extract_token_version(doc: &Document) -> Result<u64, String> {
    match doc.fields.get(TOKEN_VERSION_FIELD) {
        None => Ok(0),
        Some(Value::Int(value)) => u64::try_from(*value)
            .map_err(|_| "token_version must be a non-negative integer".to_string()),
        Some(_) => Err("token_version must be an integer".to_string()),
    }
}

fn bump_token_version(doc: &mut Document) -> Result<u64, String> {
    let next = extract_token_version(doc)?
        .checked_add(1)
        .ok_or_else(|| "token_version overflow".to_string())?;
    let stored = i64::try_from(next).map_err(|_| "token_version overflow".to_string())?;
    doc.fields
        .insert(TOKEN_VERSION_FIELD.to_string(), Value::Int(stored));
    Ok(next)
}

/// Extract user attributes from a user document into a `HashMap<String, String>`.
fn extract_attributes(doc: &Document) -> HashMap<String, String> {
    match doc.fields.get("attributes") {
        Some(Value::Object(obj)) => obj
            .iter()
            .filter_map(|(k, v)| match v {
                Value::String(s) => Some((k.clone(), s.clone())),
                _ => None,
            })
            .collect(),
        _ => HashMap::new(),
    }
}

fn optional_string_field(doc: &Document, field: &str) -> Option<String> {
    match doc.fields.get(field) {
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn managed_user_from_document(
    document: &Document,
    key_counts: &HashMap<String, usize>,
) -> Result<ManagedUser, AuthError> {
    let username = string_field(document, "username", "managed user has no username")
        .map_err(AuthError::internal)?
        .to_string();
    Ok(ManagedUser {
        api_key_count: key_counts.get(&username).copied().unwrap_or(0),
        protected: username == "root",
        created_at: optional_string_field(document, CREATED_AT_FIELD),
        updated_at: optional_string_field(document, UPDATED_AT_FIELD),
        attributes: extract_attributes(document),
        username,
    })
}

fn managed_api_key_from_document(document: &Document) -> Result<ManagedApiKey, AuthError> {
    let name = string_field(document, "name", "managed API key has no name")
        .map_err(AuthError::internal)?
        .to_string();
    let username = string_field(document, "username", "managed API key has no username")
        .map_err(AuthError::internal)?
        .to_string();
    let hint = optional_string_field(document, API_KEY_LOOKUP_FIELD).map(|lookup_id| {
        let prefix: String = lookup_id.chars().take(8).collect();
        format!("stl_{prefix}…")
    });
    Ok(ManagedApiKey {
        name,
        username,
        hint,
        created_at: optional_string_field(document, CREATED_AT_FIELD),
    })
}

/// Convert a `Policy` to a `Document` for storage in the `policies` collection.
fn policy_to_document(policy: &Policy) -> Document {
    let effect_str = match policy.effect {
        Effect::Allow => "allow",
        Effect::Deny => "deny",
    };

    let conditions_value: Vec<Value> = policy
        .conditions
        .iter()
        .map(|cond| {
            let mut obj = HashMap::new();
            obj.insert("field".to_string(), Value::String(cond.field.clone()));
            obj.insert(
                "op".to_string(),
                Value::String(match cond.op {
                    ConditionOp::Eq => "eq".to_string(),
                    ConditionOp::Neq => "neq".to_string(),
                    ConditionOp::In => "in".to_string(),
                }),
            );
            match &cond.value {
                ConditionValue::Single(v) => {
                    obj.insert("value".to_string(), Value::String(v.clone()));
                }
                ConditionValue::List(vs) => {
                    obj.insert(
                        "values".to_string(),
                        Value::Array(vs.iter().map(|v| Value::String(v.clone())).collect()),
                    );
                }
            }
            Value::Object(obj)
        })
        .collect();

    let mut fields = HashMap::new();
    fields.insert("name".to_string(), Value::String(policy.name.clone()));
    fields.insert("effect".to_string(), Value::String(effect_str.to_string()));
    fields.insert("conditions".to_string(), Value::Array(conditions_value));

    Document {
        id: format!("policies:{}", policy.name),
        fields,
    }
}

/// Convert a `Document` from the `policies` collection back to a [`Policy`].
fn document_to_policy(doc: &Document) -> Result<Policy, String> {
    let name = match doc.fields.get("name") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err("missing or invalid 'name' field".to_string()),
    };

    let effect = match doc.fields.get("effect") {
        Some(Value::String(s)) => match s.as_str() {
            "allow" => Effect::Allow,
            "deny" => Effect::Deny,
            other => return Err(format!("invalid effect: '{other}'")),
        },
        _ => return Err("missing or invalid 'effect' field".to_string()),
    };

    let conditions = match doc.fields.get("conditions") {
        Some(Value::Array(arr)) => {
            let mut conds = Vec::new();
            for item in arr {
                match item {
                    Value::Object(obj) => {
                        let field = match obj.get("field") {
                            Some(Value::String(s)) => s.clone(),
                            _ => return Err("condition missing 'field'".to_string()),
                        };

                        let op = match obj.get("op") {
                            Some(Value::String(s)) => match s.as_str() {
                                "eq" => ConditionOp::Eq,
                                "neq" => ConditionOp::Neq,
                                "in" => ConditionOp::In,
                                other => return Err(format!("invalid condition op: '{other}'")),
                            },
                            _ => return Err("condition missing 'op'".to_string()),
                        };

                        let value = if let Some(Value::String(v)) = obj.get("value") {
                            ConditionValue::Single(v.clone())
                        } else if let Some(Value::Array(vs)) = obj.get("values") {
                            let list: Result<Vec<String>, String> = vs
                                .iter()
                                .map(|v| match v {
                                    Value::String(s) => Ok(s.clone()),
                                    _ => Err("condition 'values' must contain strings".to_string()),
                                })
                                .collect();
                            ConditionValue::List(list?)
                        } else {
                            return Err("condition missing 'value' or 'values'".to_string());
                        };

                        conds.push(Condition { field, op, value });
                    }
                    _ => return Err("condition must be an object".to_string()),
                }
            }
            conds
        }
        Some(_) => return Err("'conditions' must be an array".to_string()),
        None => Vec::new(),
    };

    Ok(Policy {
        name,
        conditions,
        effect,
    })
}

impl AuthorizationProvider for AuthService {
    fn authorize(&self, subject: &Subject, action: Action, resource: &Resource) -> AuthzDecision {
        AuthService::authorize(self, subject, action, resource)
    }

    fn get_collection_attributes(
        &self,
        database: &str,
        collection: &str,
    ) -> HashMap<String, String> {
        AuthService::get_collection_attributes(self, database, collection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JWT_SECRET: &[u8] = b"test-jwt-secret-at-least-32-bytes";
    const JWT_TTL: u64 = 3600;

    fn init_service() -> (AuthService, String) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let (service, root_pw) = AuthService::init(db, JWT_SECRET, JWT_TTL).unwrap();
        // Leak tempdir to keep the database alive for the test
        std::mem::forget(tmp);
        (
            service,
            root_pw.expect("first boot should return root password"),
        )
    }

    fn legacy_api_key_document(
        name: &str,
        username: &str,
        plaintext: &str,
        identity: Option<&str>,
    ) -> Document {
        let mut fields = HashMap::from([
            ("name".to_string(), Value::String(name.to_string())),
            ("username".to_string(), Value::String(username.to_string())),
            (
                "key_hash".to_string(),
                Value::String(api_key::hash_legacy_key(plaintext)),
            ),
        ]);
        if let Some(identity) = identity {
            fields.insert(
                API_KEY_USER_IDENTITY_FIELD.to_string(),
                Value::String(identity.to_string()),
            );
        }
        Document {
            id: format!("api_keys:{name}"),
            fields,
        }
    }

    fn bootstrap_root_identity(path: &std::path::Path) -> String {
        let db = Arc::new(Database::open(path).unwrap());
        let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
        let root = db.get_document("users", "root").unwrap().unwrap();
        let identity = extract_user_identity(&root).unwrap().to_string();
        drop(service);
        drop(db);
        identity
    }

    fn count_documents(db: &Database, collection: &str) -> usize {
        let mut count = 0;
        let mut after = None;
        loop {
            let (documents, cursor) = db
                .list_documents(collection, after.as_deref(), AUTH_MIGRATION_PAGE_SIZE)
                .unwrap();
            count += documents.len();
            match cursor {
                Some(cursor) => after = Some(cursor),
                None => return count,
            }
        }
    }

    // ─── Bootstrap ───

    #[test]
    fn bootstrap_creates_root_and_returns_password() {
        let (_, root_pw) = init_service();
        assert_eq!(root_pw.len(), 24, "root password should be 24 chars");
        assert!(
            root_pw.chars().all(|c| c.is_ascii_alphanumeric()),
            "root password should be alphanumeric"
        );
    }

    #[test]
    fn second_init_does_not_re_bootstrap() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let (_, first_pw) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
        assert!(first_pw.is_some());

        let (_, second_pw) = AuthService::init(db, JWT_SECRET, JWT_TTL).unwrap();
        assert!(second_pw.is_none(), "second init should not re-bootstrap");
    }

    // ─── Login ───

    #[test]
    fn login_root_with_correct_password() {
        let (service, root_pw) = init_service();
        let token = service.login("root", &root_pw).unwrap();
        assert!(!token.is_empty());
    }

    #[test]
    fn login_with_wrong_password_fails() {
        let (service, _) = init_service();
        let result = service.login("root", "wrong-password");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid credentials"));
    }

    #[test]
    fn login_nonexistent_user_fails() {
        let (service, _) = init_service();
        let result = service.login("nobody", "anything");
        assert!(result.is_err());
    }

    // ─── JWT Authentication ───

    #[test]
    fn authenticate_valid_jwt_returns_subject() {
        let (service, root_pw) = init_service();
        let token = service.login("root", &root_pw).unwrap();

        let subject = service.authenticate(&token).unwrap();
        assert_eq!(subject.user_id, "root");
    }

    #[test]
    fn authenticate_invalid_jwt_fails() {
        let (service, _) = init_service();
        let result = service.authenticate("not.a.valid.jwt");
        assert!(result.is_err());
    }

    #[test]
    fn jwt_carries_user_attributes() {
        let (service, _root_pw) = init_service();

        // Create user with attributes
        service.create_user("alice", "password123").unwrap();
        service
            .alter_user_set(
                "alice",
                HashMap::from([
                    ("role".to_string(), "admin".to_string()),
                    ("department".to_string(), "engineering".to_string()),
                ]),
            )
            .unwrap();

        let token = service.login("alice", "password123").unwrap();
        let subject = service.authenticate(&token).unwrap();
        assert_eq!(subject.user_id, "alice");
        assert_eq!(subject.attributes.get("role"), Some(&"admin".to_string()));
        assert_eq!(
            subject.attributes.get("department"),
            Some(&"engineering".to_string())
        );
    }

    // ─── API Key Authentication ───

    #[test]
    fn authenticate_valid_api_key() {
        let (service, _) = init_service();

        let key = service.create_api_key("root-key", "root").unwrap();
        assert!(key.starts_with("stl_"));

        let subject = service.authenticate(&key).unwrap();
        assert_eq!(subject.user_id, "root");
    }

    #[test]
    fn authenticate_invalid_api_key_fails() {
        let (service, _) = init_service();
        let result = service.authenticate("stl_invalid_key_that_doesnt_exist");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid API key"));
    }

    #[test]
    fn api_key_inherits_user_attributes() {
        let (service, _) = init_service();

        service.create_user("bob", "pass").unwrap();
        service
            .alter_user_set(
                "bob",
                HashMap::from([("team".to_string(), "backend".to_string())]),
            )
            .unwrap();

        let key = service.create_api_key("bob-key", "bob").unwrap();
        let subject = service.authenticate(&key).unwrap();
        assert_eq!(subject.user_id, "bob");
        assert_eq!(subject.attributes.get("team"), Some(&"backend".to_string()));
    }

    // ─── User CRUD ───

    #[test]
    fn create_and_login_user() {
        let (service, _) = init_service();

        service.create_user("alice", "secret").unwrap();
        let token = service.login("alice", "secret").unwrap();
        assert!(!token.is_empty());
    }

    #[test]
    fn create_duplicate_user_fails() {
        let (service, _) = init_service();

        service.create_user("alice", "pass1").unwrap();
        assert_eq!(
            service.create_user_typed("alice", "pass2").unwrap_err(),
            AuthError::already_exists("user", "alice")
        );
    }

    #[test]
    fn drop_user_succeeds() {
        let (service, _) = init_service();

        service.create_user("alice", "pass").unwrap();
        assert!(service.drop_user("alice").unwrap());
        // Login should fail after drop
        assert!(service.login("alice", "pass").is_err());
    }

    #[test]
    fn drop_nonexistent_user_returns_false() {
        let (service, _) = init_service();
        assert!(!service.drop_user("nobody").unwrap());
    }

    #[test]
    fn drop_root_user_is_forbidden() {
        let (service, _) = init_service();
        let result = service.drop_user("root");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("root"));
    }

    #[test]
    fn drop_user_cascades_api_keys() {
        let (service, _) = init_service();

        service.create_user("alice", "pass").unwrap();
        let key = service.create_api_key("alice-key", "alice").unwrap();

        // API key works
        assert!(service.authenticate(&key).is_ok());

        // Drop user
        service.drop_user("alice").unwrap();

        // API key should no longer work
        assert!(service.authenticate(&key).is_err());
    }

    #[test]
    fn api_key_is_bound_to_immutable_user_identity() {
        let (service, _) = init_service();
        service.create_user("alice", "pass").unwrap();
        let key = service.create_api_key("alice-key", "alice").unwrap();
        let key_doc = service
            .system_db
            .get_document("api_keys", "alice-key")
            .unwrap()
            .unwrap();
        let lookup_id = string_field(&key_doc, API_KEY_LOOKUP_FIELD, "missing lookup")
            .unwrap()
            .to_string();
        let lookup_doc = service
            .system_db
            .get_document("api_key_lookup", &lookup_id)
            .unwrap()
            .unwrap();
        let original_identity = match key_doc.fields.get(API_KEY_USER_IDENTITY_FIELD) {
            Some(Value::String(identity)) => identity.clone(),
            other => panic!("missing API key identity: {other:?}"),
        };
        assert_eq!(
            service
                .authenticate_resolved(&key)
                .unwrap()
                .principal_identity(),
            original_identity
        );

        service.drop_user("alice").unwrap();
        service.create_user("alice", "new-pass").unwrap();
        // Simulate an orphan key surviving an interrupted legacy cascade. Its immutable
        // identity must prevent it from binding to the recreated username.
        assert!(service.system_db.create_document(&key_doc).unwrap());
        assert!(service.system_db.create_document(&lookup_doc).unwrap());
        let error = service.authenticate(&key).unwrap_err();
        assert!(error.contains("identity mismatch"));
    }

    #[test]
    fn legacy_api_key_without_identity_fails_closed_after_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        bootstrap_root_identity(tmp.path());
        let plaintext = format!("stl_{}", "ab".repeat(32));
        {
            let db = Database::open(tmp.path()).unwrap();
            let legacy = legacy_api_key_document("legacy-key", "root", &plaintext, None);
            assert!(db.create_document(&legacy).unwrap());
        }

        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
        assert!(service.authenticate(&plaintext).is_err());
        assert!(
            db.get_document(
                LEGACY_API_KEY_LOOKUP_COLLECTION,
                &api_key::hash_legacy_key(&plaintext),
            )
            .unwrap()
            .is_none(),
            "legacy records without immutable identity must not be indexed"
        );
    }

    #[test]
    fn legacy_api_key_with_identity_migrates_once_after_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = bootstrap_root_identity(tmp.path());
        let plaintext = format!("stl_{}", "cd".repeat(32));
        {
            let db = Database::open(tmp.path()).unwrap();
            let legacy = legacy_api_key_document("legacy-key", "root", &plaintext, Some(&identity));
            assert!(db.create_document(&legacy).unwrap());
        }

        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
        let legacy_hash = api_key::hash_legacy_key(&plaintext);
        assert!(
            db.get_document(LEGACY_API_KEY_LOOKUP_COLLECTION, &legacy_hash)
                .unwrap()
                .is_some(),
            "startup must build the legacy lookup index before requests"
        );
        assert_eq!(service.authenticate(&plaintext).unwrap().user_id, "root");
        let migrated = db.get_document("api_keys", "legacy-key").unwrap().unwrap();
        assert!(!migrated.fields.contains_key("key_hash"));
        let lookup_id = api_key::legacy_lookup_id(&plaintext);
        assert_eq!(
            migrated.fields.get(API_KEY_LOOKUP_FIELD),
            Some(&Value::String(lookup_id.clone()))
        );
        assert!(
            db.get_document("api_key_lookup", &lookup_id)
                .unwrap()
                .is_some()
        );
        assert!(
            db.get_document(LEGACY_API_KEY_LOOKUP_COLLECTION, &legacy_hash)
                .unwrap()
                .is_none()
        );
        assert_eq!(service.authenticate(&plaintext).unwrap().user_id, "root");
    }

    #[test]
    fn drop_legacy_api_key_atomically_removes_metadata_and_index() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = bootstrap_root_identity(tmp.path());
        let plaintext = format!("stl_{}", "bc".repeat(32));
        let legacy_hash = api_key::hash_legacy_key(&plaintext);
        {
            let db = Database::open(tmp.path()).unwrap();
            assert!(
                db.create_document(&legacy_api_key_document(
                    "legacy-drop",
                    "root",
                    &plaintext,
                    Some(&identity),
                ))
                .unwrap()
            );
        }

        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
        assert!(service.drop_api_key("legacy-drop").unwrap());
        assert!(
            db.get_document("api_keys", "legacy-drop")
                .unwrap()
                .is_none()
        );
        assert!(
            db.get_document(LEGACY_API_KEY_LOOKUP_COLLECTION, &legacy_hash)
                .unwrap()
                .is_none()
        );
        assert!(service.authenticate(&plaintext).is_err());
    }

    #[test]
    fn startup_indexes_all_legacy_keys_across_pages() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = bootstrap_root_identity(tmp.path());
        let key_count = AUTH_MIGRATION_PAGE_SIZE * 3 + 7;
        let mut plaintexts = Vec::with_capacity(key_count);
        {
            let db = Database::open(tmp.path()).unwrap();
            for index in 0..key_count {
                let plaintext = format!("stl_{index:064x}");
                let legacy = legacy_api_key_document(
                    &format!("legacy-{index:04}"),
                    "root",
                    &plaintext,
                    Some(&identity),
                );
                assert!(db.create_document(&legacy).unwrap());
                plaintexts.push(plaintext);
            }
        }

        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
        assert_eq!(
            count_documents(&db, LEGACY_API_KEY_LOOKUP_COLLECTION),
            key_count
        );
        for index in [0, AUTH_MIGRATION_PAGE_SIZE, key_count - 1] {
            assert_eq!(
                service.authenticate(&plaintexts[index]).unwrap().user_id,
                "root"
            );
        }
    }

    #[test]
    fn startup_repairs_lookup_first_migration_idempotently_across_reopens() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = bootstrap_root_identity(tmp.path());
        let plaintext = format!("stl_{}", "de".repeat(32));
        let lookup_id = api_key::legacy_lookup_id(&plaintext);
        let legacy_hash = api_key::hash_legacy_key(&plaintext);
        let entry = LegacyApiKeyEntry {
            name: "partial".to_string(),
            username: "root".to_string(),
            user_identity: identity.clone(),
        };
        {
            let db = Database::open(tmp.path()).unwrap();
            assert!(
                db.create_document(&legacy_api_key_document(
                    "partial",
                    "root",
                    &plaintext,
                    Some(&identity),
                ))
                .unwrap()
            );
            assert!(
                db.create_document(&api_key_lookup_document(
                    &lookup_id,
                    "partial",
                    "root",
                    &identity,
                    api_key::hash_legacy_secret(&plaintext),
                ))
                .unwrap()
            );
            assert!(
                db.create_document(&legacy_index_document(&legacy_hash, &[entry]))
                    .unwrap()
            );
        }

        for _ in 0..2 {
            let db = Arc::new(Database::open(tmp.path()).unwrap());
            let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
            let metadata = db.get_document("api_keys", "partial").unwrap().unwrap();
            assert_eq!(
                metadata.fields.get(API_KEY_LOOKUP_FIELD),
                Some(&Value::String(lookup_id.clone()))
            );
            assert!(!metadata.fields.contains_key("key_hash"));
            assert!(
                db.get_document(LEGACY_API_KEY_LOOKUP_COLLECTION, &legacy_hash)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(service.authenticate(&plaintext).unwrap().user_id, "root");
            drop(service);
            drop(db);
        }
    }

    #[test]
    fn current_api_key_lookup_is_independent_of_metadata_cardinality() {
        let (service, _) = init_service();
        for index in 0..(AUTH_MIGRATION_PAGE_SIZE * 3) {
            let decoy = Document {
                id: format!("api_keys:decoy-{index:04}"),
                fields: HashMap::from([
                    (
                        "name".to_string(),
                        Value::String(format!("decoy-{index:04}")),
                    ),
                    ("username".to_string(), Value::String("root".to_string())),
                ]),
            };
            assert!(service.system_db.create_document(&decoy).unwrap());
        }

        let key = service.create_api_key("direct-key", "root").unwrap();
        let metadata = service
            .system_db
            .get_document("api_keys", "direct-key")
            .unwrap()
            .unwrap();
        assert!(!metadata.fields.contains_key("key_hash"));
        assert_eq!(service.authenticate(&key).unwrap().user_id, "root");
    }

    // ─── Alter User ───

    #[test]
    fn alter_user_set_attributes() {
        let (service, root_pw) = init_service();

        service
            .alter_user_set(
                "root",
                HashMap::from([("role".to_string(), "admin".to_string())]),
            )
            .unwrap();

        let token = service.login("root", &root_pw).unwrap();
        let subject = service.authenticate(&token).unwrap();
        assert_eq!(subject.attributes.get("role"), Some(&"admin".to_string()));
    }

    #[test]
    fn alter_user_remove_attributes() {
        let (service, root_pw) = init_service();

        service
            .alter_user_set(
                "root",
                HashMap::from([
                    ("role".to_string(), "admin".to_string()),
                    ("team".to_string(), "core".to_string()),
                ]),
            )
            .unwrap();

        service
            .alter_user_remove("root", &["role".to_string()])
            .unwrap();

        let token = service.login("root", &root_pw).unwrap();
        let subject = service.authenticate(&token).unwrap();
        assert_eq!(subject.attributes.get("role"), None);
        assert_eq!(subject.attributes.get("team"), Some(&"core".to_string()));
    }

    #[test]
    fn alter_user_password() {
        let (service, _root_pw) = init_service();

        service.create_user("alice", "old_pass").unwrap();
        service.alter_user_password("alice", "new_pass").unwrap();

        // Old password fails
        assert!(service.login("alice", "old_pass").is_err());
        // New password works
        assert!(service.login("alice", "new_pass").is_ok());
    }

    #[test]
    fn alter_nonexistent_user_fails() {
        let (service, _) = init_service();
        assert!(service.alter_user_set("nobody", HashMap::new()).is_err());
        assert!(
            service
                .alter_user_remove("nobody", &["x".to_string()])
                .is_err()
        );
        assert!(service.alter_user_password("nobody", "pass").is_err());
    }

    // ─── API Key CRUD ───

    #[test]
    fn create_api_key_returns_stl_prefixed_key() {
        let (service, _) = init_service();
        let key = service.create_api_key("my-key", "root").unwrap();
        assert!(key.starts_with("stl_"));
        assert_eq!(key.len(), 4 + 32 + 1 + 64);
        let api_key::ParsedApiKey::Direct { lookup_id, secret } =
            api_key::parse_api_key(&key).unwrap()
        else {
            unreachable!();
        };
        let metadata = service
            .system_db
            .get_document("api_keys", "my-key")
            .unwrap()
            .unwrap();
        assert!(
            !metadata
                .fields
                .values()
                .any(|value| value == &Value::String(secret.to_string()))
        );
        let lookup = service
            .system_db
            .get_document("api_key_lookup", &lookup_id)
            .unwrap()
            .unwrap();
        assert!(
            !lookup
                .fields
                .values()
                .any(|value| value == &Value::String(secret.to_string()))
        );
        assert!(lookup.fields.contains_key(API_KEY_SECRET_HASH_FIELD));
    }

    #[test]
    fn create_api_key_for_nonexistent_user_fails() {
        let (service, _) = init_service();
        let result = service.create_api_key("key1", "nobody");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn create_duplicate_api_key_fails() {
        let (service, _) = init_service();
        service.create_api_key("my-key", "root").unwrap();
        let result = service.create_api_key("my-key", "root");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn create_api_key_collision_is_visible_and_leaves_no_metadata() {
        let (service, _) = init_service();
        let plaintext = format!("stl_{}_{}", "11".repeat(16), "22".repeat(32));
        let api_key::ParsedApiKey::Direct { lookup_id, .. } =
            api_key::parse_api_key(&plaintext).unwrap()
        else {
            unreachable!();
        };
        let collision = api_key_lookup_document(
            &lookup_id,
            "other-key",
            "root",
            "other-identity",
            "00".repeat(32),
        );
        assert!(service.system_db.create_document(&collision).unwrap());

        let error = service
            .create_api_key_with_plaintext_typed("new-key", "root", plaintext)
            .unwrap_err();
        assert!(matches!(error, AuthError::Internal(ref message) if message.contains("collision")));
        assert!(
            service
                .system_db
                .get_document("api_keys", "new-key")
                .unwrap()
                .is_none(),
            "lookup collision must roll back metadata creation"
        );
    }

    #[test]
    fn create_api_key_batch_failure_rolls_back_both_documents() {
        let (service, _) = init_service();
        let root = service
            .system_db
            .get_document("users", "root")
            .unwrap()
            .unwrap();
        let identity = extract_user_identity(&root).unwrap();
        let seed = api_key_lookup_document("seed", "seed-key", "root", identity, "00".repeat(32));
        assert!(service.system_db.create_document(&seed).unwrap());
        let index = crate::schema::IndexDef::unique("api_key_lookup", vec!["username".to_string()]);
        service
            .system_db
            .build_index("api_key_lookup", &index)
            .unwrap();
        service
            .system_db
            .index_registry()
            .register("api_key_lookup", index.clone());
        service
            .system_db
            .add_index_to_schema("api_key_lookup", index)
            .unwrap();

        assert!(matches!(
            service.create_api_key_typed("must-rollback", "root"),
            Err(AuthError::Internal(_))
        ));
        assert!(
            service
                .system_db
                .get_document("api_keys", "must-rollback")
                .unwrap()
                .is_none()
        );
        assert_eq!(count_documents(&service.system_db, "api_key_lookup"), 1);
    }

    #[test]
    fn legacy_migration_batch_failure_preserves_legacy_state() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = bootstrap_root_identity(tmp.path());
        let plaintext = format!("stl_{}", "ef".repeat(32));
        let lookup_id = api_key::legacy_lookup_id(&plaintext);
        {
            let db = Database::open(tmp.path()).unwrap();
            assert!(
                db.create_document(&legacy_api_key_document(
                    "legacy-fault",
                    "root",
                    &plaintext,
                    Some(&identity),
                ))
                .unwrap()
            );
            let seed =
                api_key_lookup_document("seed", "seed-key", "root", &identity, "00".repeat(32));
            assert!(db.create_document(&seed).unwrap());
            let index =
                crate::schema::IndexDef::unique("api_key_lookup", vec!["username".to_string()]);
            db.build_index("api_key_lookup", &index).unwrap();
            db.index_registry()
                .register("api_key_lookup", index.clone());
            db.add_index_to_schema("api_key_lookup", index).unwrap();
        }

        let db = Arc::new(Database::open(tmp.path()).unwrap());
        let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();
        assert!(service.authenticate(&plaintext).is_err());
        let metadata = db
            .get_document("api_keys", "legacy-fault")
            .unwrap()
            .unwrap();
        assert!(metadata.fields.contains_key("key_hash"));
        assert!(!metadata.fields.contains_key(API_KEY_LOOKUP_FIELD));
        assert!(
            db.get_document("api_key_lookup", &lookup_id)
                .unwrap()
                .is_none()
        );
        assert!(
            db.get_document(
                LEGACY_API_KEY_LOOKUP_COLLECTION,
                &api_key::hash_legacy_key(&plaintext),
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn drop_api_key_succeeds() {
        let (service, _) = init_service();
        let key = service.create_api_key("my-key", "root").unwrap();

        let metadata = service
            .system_db
            .get_document("api_keys", "my-key")
            .unwrap()
            .unwrap();
        let lookup_id = string_field(&metadata, API_KEY_LOOKUP_FIELD, "missing lookup")
            .unwrap()
            .to_string();
        assert!(service.drop_api_key("my-key").unwrap());

        assert!(service.authenticate(&key).is_err());
        assert!(
            service
                .system_db
                .get_document("api_keys", "my-key")
                .unwrap()
                .is_none()
        );
        assert!(
            service
                .system_db
                .get_document("api_key_lookup", &lookup_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn drop_nonexistent_api_key_returns_false() {
        let (service, _) = init_service();
        assert!(!service.drop_api_key("nonexistent").unwrap());
    }

    // ─── Policy CRUD ───

    #[test]
    fn create_and_evaluate_policy() {
        let (service, _) = init_service();

        let policy = Policy {
            name: "allow_read".to_string(),
            conditions: vec![Condition {
                field: "action".to_string(),
                op: ConditionOp::Eq,
                value: ConditionValue::Single("SELECT".to_string()),
            }],
            effect: Effect::Allow,
        };

        service.create_policy(&policy).unwrap();

        let subject = Subject {
            user_id: "alice".to_string(),
            attributes: HashMap::new(),
        };
        let resource = Resource {
            database: "test".to_string(),
            collection: Some("users".to_string()),
            attributes: HashMap::new(),
        };

        // SELECT allowed by policy
        assert!(matches!(
            service.authorize(&subject, Action::Select, &resource),
            AuthzDecision::Allowed
        ));

        // INSERT denied (no matching policy)
        assert!(matches!(
            service.authorize(&subject, Action::Insert, &resource),
            AuthzDecision::Denied { .. }
        ));
    }

    #[test]
    fn create_duplicate_policy_fails() {
        let (service, _) = init_service();

        let policy = Policy {
            name: "test".to_string(),
            conditions: vec![],
            effect: Effect::Allow,
        };

        service.create_policy(&policy).unwrap();
        let result = service.create_policy(&policy);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn drop_policy_removes_from_engine_and_storage() {
        let (service, _) = init_service();

        let policy = Policy {
            name: "test".to_string(),
            conditions: vec![Condition {
                field: "action".to_string(),
                op: ConditionOp::Eq,
                value: ConditionValue::Single("SELECT".to_string()),
            }],
            effect: Effect::Allow,
        };

        service.create_policy(&policy).unwrap();

        let subject = Subject {
            user_id: "alice".to_string(),
            attributes: HashMap::new(),
        };
        let resource = Resource {
            database: "test".to_string(),
            collection: None,
            attributes: HashMap::new(),
        };

        // Before drop: allowed
        assert!(matches!(
            service.authorize(&subject, Action::Select, &resource),
            AuthzDecision::Allowed
        ));

        // Drop
        assert!(service.drop_policy("test").unwrap());

        // After drop: denied (default deny)
        assert!(matches!(
            service.authorize(&subject, Action::Select, &resource),
            AuthzDecision::Denied { .. }
        ));
    }

    #[test]
    fn drop_nonexistent_policy_returns_false() {
        let (service, _) = init_service();
        assert!(!service.drop_policy("nonexistent").unwrap());
    }

    // ─── Policy persistence (reload_policies) ───

    #[test]
    fn policies_survive_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(tmp.path()).unwrap());

        let (service, _) = AuthService::init(db.clone(), JWT_SECRET, JWT_TTL).unwrap();

        service
            .create_policy(&Policy {
                name: "allow_select".to_string(),
                conditions: vec![Condition {
                    field: "action".to_string(),
                    op: ConditionOp::Eq,
                    value: ConditionValue::Single("SELECT".to_string()),
                }],
                effect: Effect::Allow,
            })
            .unwrap();

        // Re-init (simulates restart)
        let (service2, _) = AuthService::init(db, JWT_SECRET, JWT_TTL).unwrap();

        let subject = Subject {
            user_id: "alice".to_string(),
            attributes: HashMap::new(),
        };
        let resource = Resource {
            database: "test".to_string(),
            collection: None,
            attributes: HashMap::new(),
        };

        // Policy should still be active after reload
        assert!(matches!(
            service2.authorize(&subject, Action::Select, &resource),
            AuthzDecision::Allowed
        ));
    }

    // ─── policy_to_document / document_to_policy roundtrip ───

    #[test]
    fn policy_document_roundtrip_single_value() {
        let policy = Policy {
            name: "test_policy".to_string(),
            conditions: vec![
                Condition {
                    field: "subject.role".to_string(),
                    op: ConditionOp::Eq,
                    value: ConditionValue::Single("admin".to_string()),
                },
                Condition {
                    field: "action".to_string(),
                    op: ConditionOp::Neq,
                    value: ConditionValue::Single("DROP".to_string()),
                },
            ],
            effect: Effect::Allow,
        };

        let doc = policy_to_document(&policy);
        let roundtripped = document_to_policy(&doc).unwrap();

        assert_eq!(roundtripped.name, policy.name);
        assert_eq!(roundtripped.effect, policy.effect);
        assert_eq!(roundtripped.conditions.len(), 2);
        assert_eq!(roundtripped.conditions[0].field, "subject.role");
        assert_eq!(roundtripped.conditions[0].op, ConditionOp::Eq);
        assert_eq!(roundtripped.conditions[1].op, ConditionOp::Neq);
    }

    #[test]
    fn policy_document_roundtrip_list_values() {
        let policy = Policy {
            name: "multi_action".to_string(),
            conditions: vec![Condition {
                field: "action".to_string(),
                op: ConditionOp::In,
                value: ConditionValue::List(vec![
                    "SELECT".to_string(),
                    "INSERT".to_string(),
                    "UPDATE".to_string(),
                ]),
            }],
            effect: Effect::Deny,
        };

        let doc = policy_to_document(&policy);
        let roundtripped = document_to_policy(&doc).unwrap();

        assert_eq!(roundtripped.name, "multi_action");
        assert_eq!(roundtripped.effect, Effect::Deny);
        assert_eq!(roundtripped.conditions[0].op, ConditionOp::In);
        match &roundtripped.conditions[0].value {
            ConditionValue::List(v) => assert_eq!(v.len(), 3),
            _ => panic!("expected ConditionValue::List"),
        }
    }

    #[test]
    fn document_to_policy_rejects_invalid_data() {
        // Missing name
        let doc = Document {
            id: "policies:bad".to_string(),
            fields: HashMap::from([("effect".to_string(), Value::String("allow".to_string()))]),
        };
        assert!(document_to_policy(&doc).is_err());

        // Invalid effect
        let doc = Document {
            id: "policies:bad".to_string(),
            fields: HashMap::from([
                ("name".to_string(), Value::String("bad".to_string())),
                ("effect".to_string(), Value::String("maybe".to_string())),
            ]),
        };
        assert!(document_to_policy(&doc).is_err());

        // Invalid condition op
        let doc = Document {
            id: "policies:bad".to_string(),
            fields: HashMap::from([
                ("name".to_string(), Value::String("bad".to_string())),
                ("effect".to_string(), Value::String("allow".to_string())),
                (
                    "conditions".to_string(),
                    Value::Array(vec![Value::Object(HashMap::from([
                        ("field".to_string(), Value::String("action".to_string())),
                        ("op".to_string(), Value::String("LIKE".to_string())),
                        ("value".to_string(), Value::String("x".to_string())),
                    ]))]),
                ),
            ]),
        };
        assert!(document_to_policy(&doc).is_err());
    }

    // ─── Authorization ───

    #[test]
    fn root_bypasses_all_authorization() {
        let (service, _) = init_service();

        let root = Subject {
            user_id: "root".to_string(),
            attributes: HashMap::new(),
        };
        let resource = Resource {
            database: "any".to_string(),
            collection: Some("any".to_string()),
            attributes: HashMap::new(),
        };

        // No policies loaded, but root bypasses
        assert!(matches!(
            service.authorize(&root, Action::Select, &resource),
            AuthzDecision::Allowed
        ));
        assert!(matches!(
            service.authorize(&root, Action::Manage, &resource),
            AuthzDecision::Allowed
        ));
    }

    // ─── Collection Attributes ───

    #[test]
    fn set_and_get_collection_attributes() {
        let (service, _) = init_service();

        service
            .set_collection_attributes(
                "mydb",
                "users",
                HashMap::from([
                    ("sensitivity".to_string(), "pii".to_string()),
                    ("team".to_string(), "backend".to_string()),
                ]),
            )
            .unwrap();

        let attrs = service.get_collection_attributes("mydb", "users");
        assert_eq!(attrs.get("sensitivity"), Some(&"pii".to_string()));
        assert_eq!(attrs.get("team"), Some(&"backend".to_string()));
    }

    #[test]
    fn set_collection_attributes_merges_with_existing() {
        let (service, _) = init_service();

        service
            .set_collection_attributes(
                "mydb",
                "users",
                HashMap::from([("sensitivity".to_string(), "pii".to_string())]),
            )
            .unwrap();

        // Add another attribute — should merge, not replace
        service
            .set_collection_attributes(
                "mydb",
                "users",
                HashMap::from([("team".to_string(), "backend".to_string())]),
            )
            .unwrap();

        let attrs = service.get_collection_attributes("mydb", "users");
        assert_eq!(attrs.get("sensitivity"), Some(&"pii".to_string()));
        assert_eq!(attrs.get("team"), Some(&"backend".to_string()));
    }

    #[test]
    fn set_collection_attributes_overwrites_existing_key() {
        let (service, _) = init_service();

        service
            .set_collection_attributes(
                "mydb",
                "users",
                HashMap::from([("sensitivity".to_string(), "pii".to_string())]),
            )
            .unwrap();

        service
            .set_collection_attributes(
                "mydb",
                "users",
                HashMap::from([("sensitivity".to_string(), "public".to_string())]),
            )
            .unwrap();

        let attrs = service.get_collection_attributes("mydb", "users");
        assert_eq!(attrs.get("sensitivity"), Some(&"public".to_string()));
    }

    #[test]
    fn remove_collection_attributes_removes_specified_keys() {
        let (service, _) = init_service();

        service
            .set_collection_attributes(
                "mydb",
                "users",
                HashMap::from([
                    ("sensitivity".to_string(), "pii".to_string()),
                    ("team".to_string(), "backend".to_string()),
                ]),
            )
            .unwrap();

        service
            .remove_collection_attributes("mydb", "users", &["sensitivity".to_string()])
            .unwrap();

        let attrs = service.get_collection_attributes("mydb", "users");
        assert_eq!(attrs.get("sensitivity"), None);
        assert_eq!(attrs.get("team"), Some(&"backend".to_string()));
    }

    #[test]
    fn remove_collection_attributes_on_nonexistent_is_noop() {
        let (service, _) = init_service();

        // Should not error even though no attributes exist
        service
            .remove_collection_attributes("mydb", "nonexistent", &["foo".to_string()])
            .unwrap();
    }

    #[test]
    fn get_nonexistent_collection_attributes_returns_empty() {
        let (service, _) = init_service();

        let attrs = service.get_collection_attributes("mydb", "nonexistent");
        assert!(attrs.is_empty());
    }

    #[test]
    fn collection_attributes_are_per_database_and_collection() {
        let (service, _) = init_service();

        service
            .set_collection_attributes(
                "db1",
                "users",
                HashMap::from([("sensitivity".to_string(), "pii".to_string())]),
            )
            .unwrap();

        service
            .set_collection_attributes(
                "db2",
                "users",
                HashMap::from([("sensitivity".to_string(), "public".to_string())]),
            )
            .unwrap();

        let attrs1 = service.get_collection_attributes("db1", "users");
        let attrs2 = service.get_collection_attributes("db2", "users");
        assert_eq!(attrs1.get("sensitivity"), Some(&"pii".to_string()));
        assert_eq!(attrs2.get("sensitivity"), Some(&"public".to_string()));
    }

    #[test]
    fn legacy_stored_users_are_migrated_before_issuing_tokens() {
        let (service, _) = init_service();
        service.create_user("legacy", "pass").unwrap();
        let mut user = service
            .system_db
            .get_document("users", "legacy")
            .unwrap()
            .unwrap();
        user.fields.remove(USER_IDENTITY_FIELD);
        user.fields.remove(TOKEN_VERSION_FIELD);
        service.system_db.update_document(&user).unwrap();

        let token = service.login("legacy", "pass").unwrap();
        let subject = service.authenticate(&token).unwrap();
        assert_eq!(subject.user_id, "legacy");

        let migrated = service
            .system_db
            .get_document("users", "legacy")
            .unwrap()
            .unwrap();
        assert!(extract_user_identity(&migrated).is_ok());
        assert_eq!(extract_token_version(&migrated).unwrap(), 0);
        assert!(migrated.fields.contains_key(TOKEN_VERSION_FIELD));
    }

    #[test]
    fn jwt_does_not_revive_after_user_is_dropped_and_recreated() {
        let (service, _) = init_service();
        service.create_user("recreated", "old-pass").unwrap();
        let old_token = service.login("recreated", "old-pass").unwrap();

        assert!(service.drop_user("recreated").unwrap());
        service.create_user("recreated", "new-pass").unwrap();

        assert!(service.authenticate(&old_token).is_err());
        let new_token = service.login("recreated", "new-pass").unwrap();
        assert!(service.authenticate(&new_token).is_ok());
    }

    #[test]
    fn concurrent_user_change_reads_latest_version_and_revokes_intermediate_token() {
        let (service, _) = init_service();
        service.create_user("concurrent", "pass").unwrap();
        let service = Arc::new(service);
        let worker = service.clone();
        let guard = service.user_state_lock.write();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let change = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            worker
                .alter_user_set(
                    "concurrent",
                    HashMap::from([("second".to_string(), "present".to_string())]),
                )
                .unwrap();
        });
        started_rx.recv().unwrap();

        let mut first = service
            .system_db
            .get_document("users", "concurrent")
            .unwrap()
            .unwrap();
        if let Some(Value::Object(attributes)) = first.fields.get_mut("attributes") {
            attributes.insert("first".to_string(), Value::String("present".to_string()));
        }
        bump_token_version(&mut first).unwrap();
        update_user_document(&service.system_db, &first).unwrap();
        let intermediate_token = service
            .jwt_config
            .issue_versioned(
                "concurrent",
                &extract_attributes(&first),
                extract_user_identity(&first).unwrap(),
                extract_token_version(&first).unwrap(),
            )
            .unwrap();

        drop(guard);
        change.join().unwrap();

        let current = service
            .system_db
            .get_document("users", "concurrent")
            .unwrap()
            .unwrap();
        let attributes = extract_attributes(&current);
        assert_eq!(attributes.get("first").map(String::as_str), Some("present"));
        assert_eq!(
            attributes.get("second").map(String::as_str),
            Some("present")
        );
        assert_eq!(extract_token_version(&current).unwrap(), 2);
        assert!(service.authenticate(&intermediate_token).is_err());
    }

    #[test]
    fn legacy_jwt_without_token_version_is_rejected() {
        #[derive(serde::Serialize)]
        struct LegacyClaims {
            sub: String,
            exp: u64,
            iat: u64,
            attrs: HashMap<String, String>,
        }

        let (service, _) = init_service();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &LegacyClaims {
                sub: "root".to_string(),
                exp: now + 3600,
                iat: now,
                attrs: HashMap::new(),
            },
            &jsonwebtoken::EncodingKey::from_secret(JWT_SECRET),
        )
        .unwrap();

        assert!(service.authenticate(&token).is_err());
    }
}
