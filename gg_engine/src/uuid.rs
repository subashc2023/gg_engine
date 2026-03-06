use std::fmt;
use std::hash::{Hash, Hasher};

use rand::Rng;

/// A 64-bit universally unique identifier.
///
/// Generated via a high-quality random number generator (`rand::thread_rng`)
/// on construction. The probability of collision is low enough for game engine
/// use across multiple machines without a central authority.
///
/// Values are masked to 53 bits so they survive lossless round-trips through
/// Lua/JavaScript doubles (IEEE 754 `f64` has 53 bits of mantissa).
/// 2^53 ≈ 9 quadrillion possible values — more than sufficient.
///
/// A value of `0` is reserved as "uninitialized" / null.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Uuid(u64);

/// Mask to keep values within the 53-bit exact-integer range of f64.
const UUID_SAFE_MASK: u64 = (1u64 << 53) - 1;

impl Uuid {
    /// Generate a new random UUID (53-bit safe, guaranteed non-zero).
    pub fn new() -> Self {
        loop {
            let v = rand::rng().random::<u64>() & UUID_SAFE_MASK;
            if v != 0 {
                return Self(v);
            }
        }
    }

    /// Create a UUID from a known value (e.g. deserialization).
    /// Values above 2^53 are masked to preserve Lua/f64 safety.
    pub fn from_raw(value: u64) -> Self {
        Self(value & UUID_SAFE_MASK)
    }

    /// The raw 64-bit value.
    pub fn raw(&self) -> u64 {
        self.0
    }
}

impl Default for Uuid {
    fn default() -> Self {
        Self::new()
    }
}

impl Hash for Uuid {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<Uuid> for u64 {
    fn from(uuid: Uuid) -> u64 {
        uuid.0
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({})", self.0)
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn unique_generation() {
        let mut set = HashSet::new();
        for _ in 0..10_000 {
            assert!(set.insert(Uuid::new()));
        }
    }

    #[test]
    fn from_raw_roundtrip() {
        let uuid = Uuid::new();
        let raw = uuid.raw();
        let restored = Uuid::from_raw(raw);
        assert_eq!(uuid, restored);
    }

    #[test]
    fn zero_is_valid() {
        let uuid = Uuid::from_raw(0);
        assert_eq!(uuid.raw(), 0);
    }

    #[test]
    fn fits_in_f64_exactly() {
        // Every generated UUID must survive a round-trip through f64
        // without precision loss (required for Lua interop).
        for _ in 0..10_000 {
            let uuid = Uuid::new();
            let as_f64 = uuid.raw() as f64;
            let back = as_f64 as u64;
            assert_eq!(
                uuid.raw(),
                back,
                "UUID {} lost precision via f64",
                uuid.raw()
            );
        }
    }
}
