//! JWT token creation and verification for StellarDB authentication.
//!
//! Uses HS256 (HMAC-SHA256) for signing. Tokens carry user identity
//! and attributes used by the ABAC policy engine.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

/// JWT claims embedded in every issued token.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// User identifier (subject)
    pub sub: String,
    /// Expiry timestamp (seconds since UNIX epoch)
    pub exp: u64,
    /// Issued-at timestamp (seconds since UNIX epoch)
    pub iat: u64,
    /// User attributes for ABAC policy evaluation
    pub attrs: HashMap<String, String>,
    /// Durable immutable identity of the stored user record.
    /// Missing identities identify legacy tokens and are rejected by AuthService.
    #[serde(default)]
    pub user_identity: Option<String>,
    /// Durable user authentication version used for token revocation.
    /// Missing versions identify legacy tokens and are rejected by AuthService.
    #[serde(default)]
    pub token_version: Option<u64>,
}

/// Configuration for issuing and verifying JWT tokens.
pub struct JwtConfig {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    ttl_secs: u64,
}

impl JwtConfig {
    /// Create a new JWT configuration from a shared secret and token lifetime.
    pub fn new(secret: &[u8], ttl_secs: u64) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret),
            decoding_key: DecodingKey::from_secret(secret),
            ttl_secs,
        }
    }

    /// Issue a signed JWT token for the given user with their attributes.
    pub fn issue(
        &self,
        user_id: &str,
        attributes: &HashMap<String, String>,
    ) -> Result<String, String> {
        self.issue_versioned(user_id, attributes, user_id, 0)
    }

    /// Issue a token tied to the current durable user authentication version.
    pub fn issue_versioned(
        &self,
        user_id: &str,
        attributes: &HashMap<String, String>,
        user_identity: &str,
        token_version: u64,
    ) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("system time error: {e}"))?
            .as_secs();

        let claims = Claims {
            sub: user_id.to_string(),
            exp: now
                .checked_add(self.ttl_secs)
                .ok_or_else(|| "JWT expiry is out of range".to_string())?,
            iat: now,
            attrs: attributes.clone(),
            user_identity: Some(user_identity.to_string()),
            token_version: Some(token_version),
        };

        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(|e| format!("failed to encode JWT: {e}"))
    }

    /// Verify a JWT token and return its claims.
    pub fn verify(&self, token: &str) -> Result<Claims, String> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 0;
        validation.required_spec_claims = ["sub", "exp"].iter().map(|s| s.to_string()).collect();

        decode::<Claims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|e| format!("invalid JWT: {e}"))
    }

    /// Returns the configured token lifetime in seconds.
    pub fn ttl_secs(&self) -> u64 {
        self.ttl_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_and_verify() {
        let config = JwtConfig::new(b"test-secret-key", 3600);
        let attrs = HashMap::from([
            ("role".to_string(), "admin".to_string()),
            ("department".to_string(), "engineering".to_string()),
        ]);

        let token = config.issue("alice", &attrs).expect("should issue token");
        let claims = config.verify(&token).expect("should verify token");

        assert_eq!(claims.sub, "alice");
        assert_eq!(claims.attrs.get("role"), Some(&"admin".to_string()));
        assert_eq!(claims.user_identity.as_deref(), Some("alice"));
        assert_eq!(claims.token_version, Some(0));
        assert_eq!(
            claims.attrs.get("department"),
            Some(&"engineering".to_string())
        );
        assert!(claims.exp > claims.iat);
        assert_eq!(claims.exp - claims.iat, 3600);
    }

    #[test]
    fn test_invalid_token() {
        let config = JwtConfig::new(b"test-secret-key", 3600);
        let result = config.verify("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_secret() {
        let config1 = JwtConfig::new(b"secret-one", 3600);
        let config2 = JwtConfig::new(b"secret-two", 3600);

        let token = config1
            .issue("alice", &HashMap::new())
            .expect("should issue token");

        let result = config2.verify(&token);
        assert!(result.is_err());
    }

    #[test]
    fn test_expired_token() {
        let config = JwtConfig::new(b"test-secret-key", 0);

        let token = config
            .issue("alice", &HashMap::new())
            .expect("should issue token");

        // Sleep briefly to ensure the token's exp (= now at issue time) is in the past
        std::thread::sleep(std::time::Duration::from_secs(1));

        let result = config.verify(&token);
        assert!(result.is_err(), "expired token should fail verification");
    }
}
