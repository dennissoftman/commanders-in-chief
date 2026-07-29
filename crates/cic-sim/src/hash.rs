//! State hashing: how a desync is *seen*.
//!
//! Every subsystem folds its state into one of these per tick, and two machines compare the results.
//! The hash therefore has one requirement above all others: **identical input must produce identical
//! output on every platform, forever** — which rules out `std::hash::DefaultHasher` (explicitly
//! unspecified across releases) and anything keyed per-process. Speed matters much less: hashing is
//! once per subsystem per tick, not once per entity.
//!
//! FNV-1a is used: a published, public-domain algorithm (Fowler–Noll–Vo; the 64-bit offset basis and
//! prime below are from its specification) whose entire definition fits in four lines a reviewer can
//! check. It is not cryptographic and does not need to be — the adversary here is a floating-point
//! divergence, not an attacker forging a state that collides.
//!
//! # Floats are hashed by bit pattern
//!
//! [`StateHasher::write_f64`] hashes `to_bits()`, so `0.0` and `-0.0` hash differently. That is
//! correct for this purpose: they divide differently, so two machines holding different zeros *are*
//! diverged, and the hash exists to say so before anything else does.

/// The FNV-1a 64-bit offset basis, from the algorithm's specification.
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// The FNV-1a 64-bit prime, from the algorithm's specification.
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// A deterministic, platform-independent state hasher.
///
/// Deliberately not `std::hash::Hasher`: implementing that trait would invite `#[derive(Hash)]`
/// types into the tick hash, and a derived hash walks fields in an order the deriving code controls
/// rather than an order this crate pins. Everything hashed for determinism is written explicitly.
#[derive(Debug, Clone)]
pub struct StateHasher {
    state: u64,
}

impl StateHasher {
    /// A hasher at the offset basis.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: OFFSET_BASIS,
        }
    }

    /// Folds bytes in, one at a time, exactly as FNV-1a specifies.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= u64::from(byte);
            self.state = self.state.wrapping_mul(PRIME);
        }
    }

    /// Folds a `u64` in, little-endian.
    ///
    /// The endianness is stated because it is part of the format: two platforms must fold the same
    /// bytes in the same order, so the conversion is pinned rather than native.
    pub fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Folds an `i64` in, little-endian, two's complement.
    pub fn write_i64(&mut self, value: i64) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Folds an `f64` in by bit pattern.
    ///
    /// `to_bits` is total and exact, so this never rounds and never panics — and it distinguishes
    /// `0.0` from `-0.0`, which is deliberate: they behave differently under division, so two machines
    /// holding different zeros have genuinely diverged.
    pub fn write_f64(&mut self, value: f64) {
        self.write_u64(value.to_bits());
    }

    /// Folds text in as its UTF-8 bytes, length first.
    ///
    /// The length prefix keeps `("ab", "c")` and `("a", "bc")` distinct — without it, adjacent
    /// strings would concatenate into the same byte stream.
    pub fn write_str(&mut self, text: &str) {
        self.write_u64(text.len() as u64);
        self.write_bytes(text.as_bytes());
    }

    /// The accumulated hash.
    #[must_use]
    pub fn finish(&self) -> u64 {
        self.state
    }
}

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::StateHasher;

    #[test]
    fn matches_the_published_fnv1a_vectors() {
        // From the FNV specification's test suite. If these move, the algorithm is no longer FNV-1a
        // and every recorded hash in every replay is invalidated — so they are pinned exactly.
        let empty = StateHasher::new();
        assert_eq!(empty.finish(), 0xcbf2_9ce4_8422_2325);

        let mut a = StateHasher::new();
        a.write_bytes(b"a");
        assert_eq!(a.finish(), 0xaf63_dc4c_8601_ec8c);

        let mut foobar = StateHasher::new();
        foobar.write_bytes(b"foobar");
        assert_eq!(foobar.finish(), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn the_two_zeros_hash_differently() {
        let mut positive = StateHasher::new();
        positive.write_f64(0.0);
        let mut negative = StateHasher::new();
        negative.write_f64(-0.0);
        assert_ne!(positive.finish(), negative.finish());
    }

    #[test]
    fn adjacent_strings_do_not_concatenate() {
        let mut split_early = StateHasher::new();
        split_early.write_str("ab");
        split_early.write_str("c");
        let mut split_late = StateHasher::new();
        split_late.write_str("a");
        split_late.write_str("bc");
        assert_ne!(split_early.finish(), split_late.finish());
    }

    #[test]
    fn integer_widths_fold_their_stated_layout() {
        // The layout is little-endian by declaration, so a u64 and the same bytes must agree — this
        // pins that the conversion is the stated one rather than the platform's.
        let mut from_int = StateHasher::new();
        from_int.write_u64(0x0102_0304_0506_0708);
        let mut from_bytes = StateHasher::new();
        from_bytes.write_bytes(&[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
        assert_eq!(from_int.finish(), from_bytes.finish());
    }
}
