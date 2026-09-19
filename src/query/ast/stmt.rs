use super::expr::{Expr, OrderItem};
use super::types::{Assignment, DataSource, ObjectLiteral, Projection, Target};
use crate::schema::{DefaultValue, FieldType, IndexDef, SchemaMode};

/// A single item in FETCH clause
#[derive(Debug, Clone, PartialEq)]
pub struct FetchItem {
    /// Path to fetch: "author" or "author.company"
    pub path: Vec<String>,
    /// Optional alias: FETCH author AS author_doc
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectAst {
    pub projection: Projection,
    pub distinct: bool,
    pub value_mode: bool,
    pub source: DataSource,
    pub filter: Option<Expr>,
    pub group: Option<Vec<String>>,
    pub order: Option<Vec<OrderItem>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub fetch: Option<Vec<FetchItem>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertAst {
    pub collection: String,
    pub objects: Vec<ObjectLiteral>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateAst {
    pub target: Target,
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateAst {
    pub target: Target,
    pub filter: Option<Expr>,
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpsertAst {
    pub target: Target,
    pub assignments: Vec<Assignment>,
    pub replace: bool,
}

/// Target for DELETE statement - can delete documents or edges
#[derive(Debug, Clone, PartialEq)]
pub enum DeleteTarget {
    /// Delete document(s)
    Document(Target),
    /// Delete specific edge(s)
    Edge {
        from: Target,
        label: String,
        to: Option<Target>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteAst {
    pub target: DeleteTarget,
    pub filter: Option<Expr>,
}

/// SET statement for session configuration
#[derive(Debug, Clone, PartialEq)]
pub struct SetAst {
    pub key: String,
    pub value_nanos: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeginAst;

#[derive(Debug, Clone, PartialEq)]
pub struct CommitAst;

#[derive(Debug, Clone, PartialEq)]
pub struct RollbackAst;

/// Field definition in DDL
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDefAst {
    pub name: String,
    pub field_type: FieldType,
    pub required: bool,
    pub default: Option<DefaultValue>,
}

/// DEFINE COLLECTION statement
#[derive(Debug, Clone, PartialEq)]
pub struct DefineCollectionAst {
    pub name: String,
    pub mode: SchemaMode,
    pub fields: Vec<FieldDefAst>,
    pub indexes: Vec<IndexDef>,
}

/// DROP COLLECTION statement
#[derive(Debug, Clone, PartialEq)]
pub struct DropCollectionAst {
    pub name: String,
    pub cascade: bool,
}

/// DESCRIBE statement
#[derive(Debug, Clone, PartialEq)]
pub enum DescribeAst {
    Collection(String), // DESCRIBE COLLECTION <name>
    Collections,        // DESCRIBE COLLECTIONS
}

/// CREATE INDEX statement
#[derive(Debug, Clone, PartialEq)]
pub struct CreateIndexAst {
    pub collection: String,
    pub fields: Vec<String>,
    pub unique: bool,
    pub index_type: crate::schema::IndexType,
    pub hnsw_params: Option<crate::schema::HnswParams>,
    pub analyzer: Option<String>,
}

/// DROP INDEX statement
#[derive(Debug, Clone, PartialEq)]
pub struct DropIndexAst {
    /// Index name to drop
    pub name: String,
    /// Collection name
    pub collection: String,
}

/// REINDEX statement
#[derive(Debug, Clone, PartialEq)]
pub struct ReindexAst {
    /// Index name to reindex
    pub name: String,
    /// Collection name
    pub collection: String,
}

/// EXPLAIN statement
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainAst {
    pub analyze: bool,
    pub statement: Box<Statement>,
}

/// LET statement for variable binding
#[derive(Debug, Clone, PartialEq)]
pub struct LetAst {
    pub name: String,
    pub expr: super::expr::Expr,
}

/// Source for RELATE statement (from/to)
#[derive(Debug, Clone, PartialEq)]
pub enum RelateSource {
    /// Single record: user:alice
    Record(Target),
    /// Array of records: [user:alice, user:bob]
    Array(Vec<Target>),
    /// Subquery: (SELECT id FROM ...)
    Subquery(Box<SelectAst>),
}

/// Data to set on edge
#[derive(Debug, Clone, PartialEq)]
pub enum RelateData {
    /// SET field = value, ...
    Set(Vec<Assignment>),
    /// CONTENT {object}
    Content(ObjectLiteral),
}

/// What to return from RELATE
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ReturnClause {
    /// Return nothing
    None,
    /// Return edge before modification (for updates)
    Before,
    /// Return edge after creation/modification
    #[default]
    After,
    /// Return specific fields
    Fields(Vec<super::expr::ProjectionItem>),
}

/// RELATE statement
#[derive(Debug, Clone, PartialEq)]
pub struct RelateAst {
    /// Source node(s)
    pub from: RelateSource,
    /// Edge label
    pub label: String,
    /// Target node(s)
    pub to: RelateSource,
    /// Data to set on edge
    pub data: Option<RelateData>,
    /// What to return
    pub return_clause: ReturnClause,
}

/// DEFINE ANALYZER statement
#[derive(Debug, Clone, PartialEq)]
pub struct DefineAnalyzerAst {
    pub name: String,
    pub tokenizer: crate::schema::TokenizerConfig,
    pub filters: Vec<crate::schema::FilterConfig>,
}

/// DROP ANALYZER statement
#[derive(Debug, Clone, PartialEq)]
pub struct DropAnalyzerAst {
    pub name: String,
}

/// CREATE USER 'name' PASSWORD 'pass'
#[derive(Debug, Clone, PartialEq)]
pub struct CreateUserAst {
    pub username: String,
    pub password: String,
}

/// DROP USER 'name'
#[derive(Debug, Clone, PartialEq)]
pub struct DropUserAst {
    pub username: String,
}

/// ALTER USER 'name' (PASSWORD | SET | REMOVE)
#[derive(Debug, Clone, PartialEq)]
pub enum AlterUserAst {
    Password {
        username: String,
        password: String,
    },
    Set {
        username: String,
        attributes: Vec<(String, String)>,
    },
    Remove {
        username: String,
        attributes: Vec<String>,
    },
}

/// CREATE API KEY 'name' FOR USER 'username'
#[derive(Debug, Clone, PartialEq)]
pub struct CreateApiKeyAst {
    pub name: String,
    pub username: String,
}

/// DROP API KEY 'name'
#[derive(Debug, Clone, PartialEq)]
pub struct DropApiKeyAst {
    pub name: String,
}

/// CREATE POLICY name WHEN ... ALLOW|DENY
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePolicyAst {
    pub name: String,
    pub conditions: Vec<PolicyConditionAst>,
    pub effect: PolicyEffectAst,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyConditionAst {
    pub field: String,       // "subject.department", "action", "resource.sensitivity"
    pub op: String,          // "=", "!=", "IN"
    pub values: Vec<String>, // single value for =/!=, multiple for IN
}

#[derive(Debug, Clone, PartialEq)]
pub enum PolicyEffectAst {
    Allow,
    Deny,
}

/// DROP POLICY name
#[derive(Debug, Clone, PartialEq)]
pub struct DropPolicyAst {
    pub name: String,
}

/// ALTER COLLECTION name SET/REMOVE attributes
#[derive(Debug, Clone, PartialEq)]
pub enum AlterCollectionAttrsAst {
    Set {
        collection: String,
        attributes: Vec<(String, String)>,
    },
    Remove {
        collection: String,
        attributes: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Select(SelectAst),
    Insert(InsertAst),
    Create(CreateAst),
    Update(UpdateAst),
    Upsert(UpsertAst),
    Delete(DeleteAst),
    Relate(RelateAst),
    Begin(BeginAst),
    Commit(CommitAst),
    Rollback(RollbackAst),
    DefineCollection(DefineCollectionAst),
    DropCollection(DropCollectionAst),
    Describe(DescribeAst),
    CreateIndex(CreateIndexAst),
    DropIndex(DropIndexAst),
    Reindex(ReindexAst),
    Explain(ExplainAst),
    Let(LetAst),
    DefineAnalyzer(DefineAnalyzerAst),
    DropAnalyzer(DropAnalyzerAst),
    Set(SetAst),
    CreateUser(CreateUserAst),
    DropUser(DropUserAst),
    AlterUser(AlterUserAst),
    CreateApiKey(CreateApiKeyAst),
    DropApiKey(DropApiKeyAst),
    CreatePolicy(CreatePolicyAst),
    DropPolicy(DropPolicyAst),
    AlterCollectionAttrs(AlterCollectionAttrsAst),
    /// Expression as statement - evaluates and returns value
    Expr(super::expr::Expr),
}
