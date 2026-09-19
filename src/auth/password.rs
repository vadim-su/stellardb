//! Password hashing and verification using Argon2.

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

/// Hashes a plaintext password using Argon2id with a random salt.
///
/// Returns the PHC-formatted hash string on success.
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| format!("Failed to hash password: {e}"))
}

/// Verifies a plaintext password against a PHC-formatted Argon2 hash.
///
/// Returns `true` if the password matches, `false` otherwise.
pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(hash) else {
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let password = "hunter2";
        let hash = hash_password(password).expect("hashing should succeed");

        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong_password", &hash));
    }

    #[test]
    fn test_different_hashes_for_same_password() {
        let password = "same_password";
        let hash1 = hash_password(password).expect("hashing should succeed");
        let hash2 = hash_password(password).expect("hashing should succeed");

        // Different salts produce different hashes
        assert_ne!(hash1, hash2);

        // Both hashes still verify against the original password
        assert!(verify_password(password, &hash1));
        assert!(verify_password(password, &hash2));
    }
}
