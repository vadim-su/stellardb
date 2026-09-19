// src/query/optimize/mod.rs

pub mod access_path;
pub mod index;
pub mod optimizer;
pub mod predicate;

pub use access_path::{
    AccessMethod, AccessPath, combine_exprs, generate_access_paths, select_best,
};
pub use index::{
    FtsAnalysis, FtsInfo, IndexChoice, SkipReason, analyze_for_fts, analyze_for_index,
    analyze_or_for_union, compute_needs_dedup,
};
pub use optimizer::Optimizer;
pub use predicate::{
    Conjunct, IndexValue, IndexableOp, PredicatePart, classify_all, classify_conjunct,
    extract_conjuncts,
};
