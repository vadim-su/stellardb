//! Lock striping for fine-grained concurrent access control.
//!
//! Instead of a single global lock, uses N independent locks (stripes).
//! Operations are mapped to stripes via hashing, reducing contention
//! when different keys are accessed concurrently.

use parking_lot::{Mutex, MutexGuard};
use std::hash::{BuildHasher, Hash};
use xxhash_rust::xxh3::Xxh3DefaultBuilder;

/// Number of stripes (must be power of 2 for fast modulo)
const DEFAULT_STRIPE_COUNT: usize = 64;

/// Lock striping implementation for fine-grained concurrency control.
///
/// # Example
///
/// ```ignore
/// let stripes = LockStripes::new();
///
/// // Lock based on a key - only this stripe is locked
/// let _guard = stripes.lock("user@example.com");
///
/// // Other keys on different stripes can proceed in parallel
/// ```
pub struct LockStripes {
    stripes: Box<[Mutex<()>]>,
    mask: usize,
}

impl LockStripes {
    /// Create a new LockStripes with default stripe count (64).
    pub fn new() -> Self {
        Self::with_count(DEFAULT_STRIPE_COUNT)
    }

    /// Create a new LockStripes with specified stripe count.
    ///
    /// # Panics
    ///
    /// Panics if `count` is 0 or not a power of 2.
    pub fn with_count(count: usize) -> Self {
        assert!(count > 0, "stripe count must be positive");
        assert!(count.is_power_of_two(), "stripe count must be a power of 2");

        let stripes: Vec<Mutex<()>> = (0..count).map(|_| Mutex::new(())).collect();

        Self {
            stripes: stripes.into_boxed_slice(),
            mask: count - 1,
        }
    }

    /// Acquire a lock for the given key.
    ///
    /// Returns a guard that releases the lock when dropped.
    /// Only the stripe corresponding to this key's hash is locked.
    pub fn lock<K: Hash>(&self, key: &K) -> MutexGuard<'_, ()> {
        let stripe_idx = self.stripe_index(key);
        self.stripes[stripe_idx].lock()
    }

    /// Try to acquire a lock for the given key without blocking.
    ///
    /// Returns `Some(guard)` if the lock was acquired, `None` if it's held by another thread.
    pub fn try_lock<K: Hash>(&self, key: &K) -> Option<MutexGuard<'_, ()>> {
        let stripe_idx = self.stripe_index(key);
        self.stripes[stripe_idx].try_lock()
    }

    /// Get the stripe index for a given key.
    fn stripe_index<K: Hash>(&self, key: &K) -> usize {
        let hash = Xxh3DefaultBuilder.hash_one(key);
        (hash as usize) & self.mask
    }

    /// Get the number of stripes.
    pub fn stripe_count(&self) -> usize {
        self.stripes.len()
    }

    /// Acquire locks for multiple keys, handling the case where different keys
    /// may hash to the same stripe (which would otherwise cause deadlock).
    ///
    /// Keys are deduplicated by stripe index and locked in sorted order to
    /// prevent deadlocks between concurrent callers.
    ///
    /// Returns a vec of guards that release the locks when dropped.
    pub fn lock_multiple<K: Hash>(&self, keys: &[K]) -> Vec<MutexGuard<'_, ()>> {
        // Compute stripe indices for all keys
        let mut stripe_indices: Vec<usize> = keys.iter().map(|k| self.stripe_index(k)).collect();

        // Sort and deduplicate to avoid locking same stripe twice (deadlock)
        // and to ensure consistent ordering across threads (prevents deadlock)
        stripe_indices.sort_unstable();
        stripe_indices.dedup();

        // Acquire locks in order
        stripe_indices
            .into_iter()
            .map(|idx| self.stripes[idx].lock())
            .collect()
    }
}

impl Default for LockStripes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_basic_locking() {
        let stripes = LockStripes::new();

        let _guard = stripes.lock(&"key1");
        // Lock acquired, would block if same stripe

        // Different key might be on different stripe
        if let Some(_guard2) = stripes.try_lock(&"key2") {
            // Got lock on different stripe
        }
    }

    #[test]
    fn test_same_key_blocks() {
        let stripes = Arc::new(LockStripes::new());
        let stripes2 = Arc::clone(&stripes);

        let key = "same_key";

        // Hold lock in main thread
        let _guard = stripes.lock(&key);

        // Try to lock same key in another thread - should fail
        let handle = thread::spawn(move || stripes2.try_lock(&key).is_none());

        assert!(handle.join().unwrap(), "same key should block");
    }

    #[test]
    fn test_stripe_distribution() {
        let stripes = LockStripes::with_count(16);

        // Different keys should (mostly) go to different stripes
        let mut stripe_hits = vec![0usize; 16];
        for i in 0..1000 {
            let key = format!("key_{}", i);
            let idx = stripes.stripe_index(&key);
            stripe_hits[idx] += 1;
        }

        // Check reasonable distribution (no stripe has > 20% of keys)
        for hits in stripe_hits {
            assert!(hits < 200, "poor stripe distribution: {} hits", hits);
        }
    }

    #[test]
    fn test_concurrent_different_stripes() {
        let stripes = Arc::new(LockStripes::with_count(64));
        let mut handles = vec![];

        // Spawn threads that lock different keys
        for i in 0..10 {
            let stripes = Arc::clone(&stripes);
            handles.push(thread::spawn(move || {
                let key = format!("unique_key_{}", i);
                let _guard = stripes.lock(&key);
                // Simulate work
                thread::sleep(std::time::Duration::from_millis(1));
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    #[should_panic(expected = "power of 2")]
    fn test_invalid_stripe_count() {
        LockStripes::with_count(17);
    }

    #[test]
    #[should_panic(expected = "positive")]
    fn test_zero_stripe_count() {
        LockStripes::with_count(0);
    }
}
