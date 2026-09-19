use fjall::Readable;
use rkyv::rancor;

use crate::document::Document;
use crate::error::StorageError;

use super::{Database, ExternalIndexJob};
use crate::storage::WriteOp;

struct PreparedBatch {
    doc_inserts: Vec<PreparedDocInsert>,
    doc_deletes: Vec<PreparedDocDelete>,
    edge_inserts: Vec<crate::edge::Edge>,
    edge_deletes: Vec<(String, String, String)>,
}

struct PreparedDocInsert {
    doc: Document,
    old_doc: Option<Document>,
    collection: String,
    create_only: bool,
}

struct PreparedDocDelete {
    collection: String,
    key: String,
    old_doc: Option<Document>,
}

struct PendingDocMutation {
    collection: String,
    key: String,
    initial_doc: Option<Document>,
    current_doc: Option<Document>,
    create_only: bool,
}

impl PreparedDocInsert {
    fn constraint_old_doc(&self) -> Option<&Document> {
        if self.create_only {
            None
        } else {
            self.old_doc.as_ref()
        }
    }
}

impl PreparedBatch {
    fn new() -> Self {
        Self {
            doc_inserts: Vec::new(),
            doc_deletes: Vec::new(),
            edge_inserts: Vec::new(),
            edge_deletes: Vec::new(),
        }
    }

    fn rollback_constraints(&self, db: &Database) {
        for insert in &self.doc_inserts {
            if let Some(keyspaces) = db.keyspaces.get(&insert.collection) {
                let indexes = db.indexes.get_indexes(&insert.collection);
                db.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    insert.constraint_old_doc(),
                    Some(&insert.doc),
                );
            }
        }
        for delete in &self.doc_deletes {
            if let (Some(old_doc), Some(keyspaces)) =
                (&delete.old_doc, db.keyspaces.get(&delete.collection))
            {
                let indexes = db.indexes.get_indexes(&delete.collection);
                db.constraints.rollback_validate_and_update(
                    &keyspaces.unique_locks,
                    &indexes,
                    Some(old_doc),
                    None,
                );
            }
        }
    }
}

pub(super) fn atomic_write_batch(db: &Database, ops: Vec<WriteOp>) -> Result<(), StorageError> {
    if ops.is_empty() {
        return Ok(());
    }

    let prepared = prepare(db, ops)?;
    commit(db, &prepared)
}

fn prepare(db: &Database, ops: Vec<WriteOp>) -> Result<PreparedBatch, StorageError> {
    // Collapse each document's sequential mutations before touching unique
    // constraints or external indexes. One Fjall transaction stores only the
    // final value for a key, so post-commit index work must use the same view.
    let mut prepared = PreparedBatch::new();
    let mut mutations = Vec::<PendingDocMutation>::new();
    let mut mutation_positions = std::collections::HashMap::<(String, String), usize>::new();

    for op in ops {
        match op {
            op @ (WriteOp::InsertDoc(_) | WriteOp::CreateDoc(_)) => {
                let create_only = matches!(&op, WriteOp::CreateDoc(_));
                let doc = match op {
                    WriteOp::InsertDoc(doc) | WriteOp::CreateDoc(doc) => doc,
                    _ => unreachable!(),
                };
                let collection = doc.collection().to_string();
                let key = doc.key().to_string();
                db.get_or_create_collection(&collection)?;
                let identity = (collection.clone(), key.clone());
                let position = if let Some(position) = mutation_positions.get(&identity) {
                    *position
                } else {
                    let initial_doc = db.documents.get(&collection, &key)?;
                    let position = mutations.len();
                    mutations.push(PendingDocMutation {
                        collection,
                        key,
                        current_doc: initial_doc.clone(),
                        initial_doc,
                        create_only: false,
                    });
                    mutation_positions.insert(identity, position);
                    position
                };
                let mutation = &mut mutations[position];
                if create_only && mutation.current_doc.is_some() {
                    return Err(StorageError::AlreadyExists(doc.id));
                }
                mutation.create_only |= create_only && mutation.initial_doc.is_none();
                mutation.current_doc = Some(doc);
            }
            WriteOp::DeleteDoc { collection, key } => {
                let identity = (collection.clone(), key.clone());
                let position = if let Some(position) = mutation_positions.get(&identity) {
                    *position
                } else {
                    let initial_doc = db.documents.get(&collection, &key)?;
                    let position = mutations.len();
                    mutations.push(PendingDocMutation {
                        collection,
                        key,
                        current_doc: initial_doc.clone(),
                        initial_doc,
                        create_only: false,
                    });
                    mutation_positions.insert(identity, position);
                    position
                };
                mutations[position].current_doc = None;
            }
            WriteOp::InsertEdge(edge) => prepared.edge_inserts.push(edge),
            WriteOp::DeleteEdge { from, label, to } => {
                prepared.edge_deletes.push((from, label, to));
            }
        }
    }

    for mutation in mutations {
        match (mutation.initial_doc, mutation.current_doc) {
            (_, Some(doc)) => {
                prepare_doc_insert(db, &mut prepared, doc, mutation.create_only)?;
            }
            (Some(_), None) => {
                prepare_doc_delete(db, &mut prepared, mutation.collection, mutation.key)?;
            }
            (None, None) => {}
        }
    }

    Ok(prepared)
}

fn prepare_doc_insert(
    db: &Database,
    prepared: &mut PreparedBatch,
    doc: Document,
    create_only: bool,
) -> Result<(), StorageError> {
    let collection = doc.collection().to_string();
    let key = doc.key().to_string();

    if let Err(err) = db.get_or_create_collection(&collection) {
        prepared.rollback_constraints(db);
        return Err(err);
    }

    let old_doc = match db.documents.get(&collection, &key) {
        Ok(old_doc) => old_doc,
        Err(err) => {
            prepared.rollback_constraints(db);
            return Err(err);
        }
    };
    // Validate unique constraints
    let keyspaces = match db.keyspaces.get(&collection) {
        Some(keyspaces) => keyspaces,
        None => {
            prepared.rollback_constraints(db);
            return Err(StorageError::Other(
                "Collection not found after creation".into(),
            ));
        }
    };
    let indexes = db.indexes.get_indexes(&collection);
    let constraint_old_doc = if create_only { None } else { old_doc.as_ref() };
    if let Err(err) = db.constraints.validate_and_update(
        &keyspaces.unique_locks,
        &indexes,
        constraint_old_doc,
        Some(&doc),
    ) {
        db.constraints.rollback_validate_and_update(
            &keyspaces.unique_locks,
            &indexes,
            constraint_old_doc,
            Some(&doc),
        );
        prepared.rollback_constraints(db);
        return Err(err);
    }

    // Pre-validate HNSW vectors
    if let Err(err) = db.indexes.validate_hnsw_vectors(&collection, &doc) {
        db.constraints.rollback_validate_and_update(
            &keyspaces.unique_locks,
            &indexes,
            constraint_old_doc,
            Some(&doc),
        );
        prepared.rollback_constraints(db);
        return Err(err);
    }

    prepared.doc_inserts.push(PreparedDocInsert {
        doc,
        old_doc,
        collection,
        create_only,
    });

    Ok(())
}

fn prepare_doc_delete(
    db: &Database,
    prepared: &mut PreparedBatch,
    collection: String,
    key: String,
) -> Result<(), StorageError> {
    let old_doc = match db.documents.get(&collection, &key) {
        Ok(old_doc) => old_doc,
        Err(err) => {
            prepared.rollback_constraints(db);
            return Err(err);
        }
    };

    // Remove unique constraint entries for deleted docs
    if let (Some(old), Some(keyspaces)) = (&old_doc, db.keyspaces.get(&collection)) {
        let indexes = db.indexes.get_indexes(&collection);
        if let Err(err) = db
            .constraints
            .remove_for_document(&keyspaces.unique_locks, &indexes, old)
        {
            db.constraints.rollback_validate_and_update(
                &keyspaces.unique_locks,
                &indexes,
                Some(old),
                None,
            );
            prepared.rollback_constraints(db);
            return Err(err);
        }
    }

    prepared.doc_deletes.push(PreparedDocDelete {
        collection,
        key,
        old_doc,
    });

    Ok(())
}

fn commit(db: &Database, prepared: &PreparedBatch) -> Result<(), StorageError> {
    let mut tx = match db.db.write_tx() {
        Ok(tx) => tx,
        Err(err) => {
            prepared.rollback_constraints(db);
            return Err(err.into());
        }
    };

    write_doc_inserts(db, prepared, &mut tx)?;
    write_doc_deletes(db, prepared, &mut tx)?;
    write_edge_inserts(db, prepared, &mut tx)?;
    write_edge_deletes(db, prepared, &mut tx)?;

    #[cfg(feature = "fault-injection")]
    if let Err(error) = crate::fault_injection::check("fjall_before_commit") {
        prepared.rollback_constraints(db);
        return Err(error);
    }
    match tx.commit() {
        Err(err) => {
            prepared.rollback_constraints(db);
            Err(err.into())
        }
        Ok(Ok(())) => {
            apply_post_commit_indexes(db, prepared);
            #[cfg(feature = "fault-injection")]
            if let Err(error) = crate::fault_injection::check("primary_after_commit") {
                tracing::warn!(%error, "ignored post-commit injected fault");
            }
            Ok(())
        }
        Ok(Err(fjall::Conflict)) => {
            prepared.rollback_constraints(db);
            Err(StorageError::Conflict("batch write conflict".into()))
        }
    }
}

fn write_doc_inserts(
    db: &Database,
    prepared: &PreparedBatch,
    tx: &mut fjall::OptimisticWriteTx,
) -> Result<(), StorageError> {
    for insert in &prepared.doc_inserts {
        let keyspaces = match db.keyspaces.get(&insert.collection) {
            Some(keyspaces) => keyspaces,
            None => {
                prepared.rollback_constraints(db);
                return Err(StorageError::CollectionNotLoaded(
                    "collection not found".into(),
                ));
            }
        };

        if insert.create_only {
            match tx.get(&keyspaces.docs, insert.doc.key().as_bytes()) {
                Ok(Some(_)) => {
                    prepared.rollback_constraints(db);
                    return Err(StorageError::AlreadyExists(insert.doc.id.clone()));
                }
                Ok(None) => {}
                Err(err) => {
                    prepared.rollback_constraints(db);
                    return Err(err.into());
                }
            }
        }

        let bytes = match rkyv::to_bytes::<rancor::Error>(&insert.doc) {
            Ok(bytes) => bytes,
            Err(err) => {
                prepared.rollback_constraints(db);
                return Err(StorageError::Serialization(err.to_string()));
            }
        };

        tx.insert(
            &keyspaces.docs,
            insert.doc.key().as_bytes(),
            bytes.as_slice(),
        );

        if insert.old_doc.is_none() {
            tx.insert(&keyspaces.exists, insert.doc.key().as_bytes(), []);
        }

        if let Err(err) = db.indexes.update_btree_in_tx(
            tx,
            &insert.collection,
            insert.old_doc.as_ref(),
            Some(&insert.doc),
        ) {
            prepared.rollback_constraints(db);
            return Err(err);
        }

        if let Err(err) = db.write_external_index_job(
            tx,
            &insert.collection,
            insert.doc.key(),
            insert.old_doc.as_ref(),
            Some(&insert.doc),
        ) {
            prepared.rollback_constraints(db);
            return Err(err);
        }
    }

    Ok(())
}

fn write_doc_deletes(
    db: &Database,
    prepared: &PreparedBatch,
    tx: &mut fjall::OptimisticWriteTx,
) -> Result<(), StorageError> {
    for delete in &prepared.doc_deletes {
        if let Some(keyspaces) = db.keyspaces.get(&delete.collection) {
            tx.remove(&keyspaces.docs, delete.key.as_bytes());
            tx.remove(&keyspaces.exists, delete.key.as_bytes());

            if let Some(ref old_doc) = delete.old_doc {
                if let Err(err) =
                    db.indexes
                        .update_btree_in_tx(tx, &delete.collection, Some(old_doc), None)
                {
                    prepared.rollback_constraints(db);
                    return Err(err);
                }
                if let Err(err) = db.write_external_index_job(
                    tx,
                    &delete.collection,
                    &delete.key,
                    Some(old_doc),
                    None,
                ) {
                    prepared.rollback_constraints(db);
                    return Err(err);
                }
            }
        }
    }

    Ok(())
}

fn write_edge_inserts(
    db: &Database,
    prepared: &PreparedBatch,
    tx: &mut fjall::OptimisticWriteTx,
) -> Result<(), StorageError> {
    for edge in &prepared.edge_inserts {
        if let Err(err) = db.validate_edge_endpoints_in_tx(tx, edge) {
            prepared.rollback_constraints(db);
            return Err(err);
        }
        if let Err(err) = db.edges.create_in_tx(tx, edge) {
            prepared.rollback_constraints(db);
            return Err(err);
        }
    }

    Ok(())
}

fn write_edge_deletes(
    db: &Database,
    prepared: &PreparedBatch,
    tx: &mut fjall::OptimisticWriteTx,
) -> Result<(), StorageError> {
    for (from, label, to) in &prepared.edge_deletes {
        if let Err(err) = db.edges.delete_in_tx(tx, from, label, to) {
            prepared.rollback_constraints(db);
            return Err(err);
        }
    }

    Ok(())
}

fn apply_post_commit_indexes(db: &Database, prepared: &PreparedBatch) {
    let mut jobs = Vec::with_capacity(prepared.doc_inserts.len() + prepared.doc_deletes.len());

    for insert in &prepared.doc_inserts {
        if db.has_external_indexes(&insert.collection) {
            jobs.push(ExternalIndexJob {
                collection: insert.collection.clone(),
                key: insert.doc.key().to_string(),
                old_doc: insert.old_doc.clone(),
                new_doc: Some(insert.doc.clone()),
            });
        }
    }

    for delete in &prepared.doc_deletes {
        if db.has_external_indexes(&delete.collection)
            && let Some(ref old_doc) = delete.old_doc
        {
            jobs.push(ExternalIndexJob {
                collection: delete.collection.clone(),
                key: delete.key.clone(),
                old_doc: Some(old_doc.clone()),
                new_doc: None,
            });
        }
    }

    db.try_apply_and_complete_external_index_jobs(&jobs);
}
