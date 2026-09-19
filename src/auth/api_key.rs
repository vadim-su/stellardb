//! API key generation, parsing, and hashing for StellarDB.

use rand::RngExt;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const API_KEY_PREFIX: &str = "stl_";
const LOOKUP_ID_BYTES: usize = 16;
const API_KEY_SECRET_BYTES: usize = 32;
const LEGACY_KEY_BYTES: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedApiKey<'a> {
    Direct { lookup_id: String, secret: &'a str },
    Legacy,
}

/// Generates a key as `stl_<lookup-id>_<secret>`.
///
/// The random lookup ID permits direct storage lookup. The secret is returned
/// only as part of this value and must never be persisted in plaintext.
pub fn generate_api_key() -> String {
    let mut lookup_id = [0u8; LOOKUP_ID_BYTES];
    let mut secret = [0u8; API_KEY_SECRET_BYTES];
    rand::rng().fill(&mut lookup_id);
    rand::rng().fill(&mut secret);

    format!(
        "{}{}_{}",
        API_KEY_PREFIX,
        hex::encode(lookup_id),
        hex::encode(secret)
    )
}

/// Parses both the current direct-lookup format and the old `stl_<secret>` format.
pub fn parse_api_key(token: &str) -> Option<ParsedApiKey<'_>> {
    let payload = token.strip_prefix(API_KEY_PREFIX)?;
    if let Some((lookup_id, secret)) = payload.split_once('_') {
        if lookup_id.len() != LOOKUP_ID_BYTES * 2
            || secret.len() != API_KEY_SECRET_BYTES * 2
            || !lookup_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !secret.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        let normalized_lookup_id = hex::encode(hex::decode(lookup_id).ok()?);
        return Some(ParsedApiKey::Direct {
            lookup_id: normalized_lookup_id,
            secret,
        });
    }

    if payload.len() == LEGACY_KEY_BYTES * 2 && payload.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Some(ParsedApiKey::Legacy)
    } else {
        None
    }
}

/// Hashes a current API key secret with SHA-256 for storage.
pub fn hash_secret(secret: &str) -> Option<String> {
    let bytes = hex::decode(secret).ok()?;
    if bytes.len() != API_KEY_SECRET_BYTES {
        return None;
    }
    Some(hex::encode(Sha256::digest(bytes)))
}

/// Compares a presented secret with a stored SHA-256 hash in constant time.
pub fn verify_secret(secret: &str, stored_hash: &str) -> bool {
    let Some(expected) = hash_secret(secret).and_then(|hash| hex::decode(hash).ok()) else {
        return false;
    };
    constant_time_hash_eq(&expected, stored_hash)
}

/// Returns a stable direct lookup ID for a legacy key after bounded migration.
pub fn legacy_lookup_id(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    hex::encode(&digest[..LOOKUP_ID_BYTES])
}

/// Returns a cryptographic storage hash for the complete legacy key.
pub fn hash_legacy_secret(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// Verifies a migrated legacy key in constant time.
pub fn verify_legacy_secret(key: &str, stored_hash: &str) -> bool {
    constant_time_hash_eq(&Sha256::digest(key.as_bytes()), stored_hash)
}

/// Computes the hash used by pre-direct-lookup StellarDB releases.
///
/// This remains only for bounded legacy migration and must not be used for new keys.
pub fn hash_legacy_key(key: &str) -> String {
    let hash = xxhash_rust::xxh3::xxh3_128(key.as_bytes());
    hex::encode(hash.to_be_bytes())
}

/// Compares a legacy lookup hash in constant time.
pub fn verify_legacy_lookup_hash(key: &str, stored_hash: &str) -> bool {
    let expected = xxhash_rust::xxh3::xxh3_128(key.as_bytes()).to_be_bytes();
    constant_time_hash_eq(&expected, stored_hash)
}

fn constant_time_hash_eq(expected: &[u8], stored_hex: &str) -> bool {
    let Ok(stored) = hex::decode(stored_hex) else {
        return false;
    };
    expected.ct_eq(stored.as_slice()).into()
}

/// Returns `true` if the token uses the StellarDB API key prefix.
pub fn is_api_key(token: &str) -> bool {
    token.starts_with(API_KEY_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_lookup_id_and_secret() {
        let key = generate_api_key();
        let ParsedApiKey::Direct { lookup_id, secret } = parse_api_key(&key).unwrap() else {
            panic!("generated key should use direct format");
        };
        assert_eq!(lookup_id.len(), LOOKUP_ID_BYTES * 2);
        assert_eq!(secret.len(), API_KEY_SECRET_BYTES * 2);
        assert_eq!(key.len(), 4 + 32 + 1 + 64);
    }

    #[test]
    fn generated_keys_are_unique() {
        assert_ne!(generate_api_key(), generate_api_key());
    }

    #[test]
    fn secret_hash_is_deterministic_and_verifiable() {
        let key = generate_api_key();
        let ParsedApiKey::Direct { secret, .. } = parse_api_key(&key).unwrap() else {
            unreachable!();
        };
        let hash = hash_secret(secret).unwrap();
        assert_eq!(hash, hash_secret(secret).unwrap());
        assert!(verify_secret(secret, &hash));
        assert!(!verify_secret(&"00".repeat(API_KEY_SECRET_BYTES), &hash));
    }

    #[test]
    fn parser_rejects_malformed_direct_keys() {
        assert!(parse_api_key("stl_bad_secret").is_none());
        assert!(parse_api_key(&format!("stl_{}_{}", "g".repeat(32), "0".repeat(64))).is_none());
        assert!(parse_api_key("stl_abc123").is_none());
    }

    #[test]
    fn parser_accepts_legacy_format() {
        let key = format!("stl_{}", "ab".repeat(LEGACY_KEY_BYTES));
        assert_eq!(parse_api_key(&key), Some(ParsedApiKey::Legacy));
    }

    #[test]
    fn api_key_prefix_detection_is_compatible() {
        assert!(is_api_key("stl_abcdef1234567890"));
        assert!(!is_api_key("not-a-key"));
        assert!(!is_api_key(""));
    }
}
