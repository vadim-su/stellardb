use std::collections::HashMap;
use std::time::Duration;

use crate::document::Value;
use crate::query::execute::VarScope;
use crate::query::result::StatementResult;

pub(super) struct BatchState {
    pub(super) results: Vec<StatementResult>,
    pub(super) vars: VarScope,
    pub(super) batch_tx: Option<String>,
    pub(super) batch_query_timeout: Option<Option<Duration>>,
}

impl BatchState {
    pub(super) fn new(params: HashMap<String, Value>) -> Self {
        Self {
            results: Vec::new(),
            vars: VarScope::with_params(params),
            batch_tx: None,
            batch_query_timeout: None,
        }
    }
}
