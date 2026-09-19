use axum::Json;
use serde::Serialize;

/// Versioned server capability contract consumed by Station and API clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesResponse {
    pub api_version: &'static str,
    pub version: &'static str,
    pub features: ServerFeatures,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerFeatures {
    pub explain: bool,
    pub explain_analyze: bool,
    pub query_cancellation: bool,
    pub audit_log: bool,
    pub schema_mutations: bool,
    pub user_management: bool,
    pub role_management: bool,
    pub api_key_management: bool,
    pub import_jobs: bool,
    pub temporal_metrics: bool,
    pub saved_queries: bool,
}

impl CapabilitiesResponse {
    pub const fn current() -> Self {
        Self {
            api_version: "v1",
            version: env!("CARGO_PKG_VERSION"),
            features: ServerFeatures {
                explain: true,
                explain_analyze: true,
                // Aborting an HTTP request does not currently cancel server execution.
                query_cancellation: false,
                audit_log: false,
                // DEFINE/DROP COLLECTION and index DDL are supported.
                schema_mutations: true,
                // Available through authenticated SQL statements.
                user_management: true,
                // StellarDB uses ABAC policies rather than role resources.
                role_management: false,
                // Available through authenticated SQL statements.
                api_key_management: true,
                import_jobs: false,
                temporal_metrics: false,
                saved_queries: false,
            },
        }
    }
}

/// GET /v1/capabilities
pub async fn capabilities_handler() -> Json<CapabilitiesResponse> {
    Json(CapabilitiesResponse::current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_the_public_camel_case_contract() {
        let value = serde_json::to_value(CapabilitiesResponse::current()).unwrap();

        assert_eq!(value["apiVersion"], "v1");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["features"]["explain"], true);
        assert_eq!(value["features"]["queryCancellation"], false);
        assert_eq!(value["features"]["schemaMutations"], true);
    }
}
