//! Sessions, memory notes, checkpoints and trust records. See docs/plan/07-sessions-memory-checkpoints.md.

pub mod checkpoint;
pub mod memory;
pub mod session;
pub mod time;
pub mod trust;

/// FNV-1a 64-bit hash; stable across builds. Used to detect changes (the project key
/// names a folder on disk, the undo hash notices re-edits), so the constants and the byte
/// order must stay the same. Not for security.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}
