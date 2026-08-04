//! Deterministic router-side Actor owner selection (E-actor-parity).
//!
//! The owner is pinned with `sha256(actorIdHash)` big-endian first four bytes
//! modulo the sorted registered candidate count (TS coordinator parity). Both
//! the ordinary actor lane (`ActorFrameSink`) and the durable task
//! actor-method admission lane consume the same selector so get-or-activate
//! and `std.actor.get` converge on the same owner for one logical key.

use sha2::{Digest, Sha256};

use crate::session::identity::RuntimeSessionEpoch;

/// Deterministic owner selection: `sha256(actorIdHash)` big-endian first four
/// bytes modulo the sorted candidate count. Empty candidates never select an
/// owner.
pub(crate) fn pick_owner_candidate<'a>(
    candidates: &'a [RuntimeSessionEpoch],
    actor_id_hash: &str,
) -> Option<&'a RuntimeSessionEpoch> {
    if candidates.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(actor_id_hash.as_bytes());
    let digest = hasher.finalize();
    let index = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) as usize
        % candidates.len();
    candidates.get(index)
}

#[cfg(test)]
mod tests {
    use super::pick_owner_candidate;
    use crate::session::identity::RuntimeSessionEpoch;

    #[test]
    fn owner_selection_pins_ts_hash_modulo_candidates() {
        // E-actor-parity: the Router pins the owner with
        // sha256(actorIdHash) big-endian first 4 bytes modulo the sorted
        // candidate count (TS coordinator pickOwner parity).
        let session = |replica: &str| RuntimeSessionEpoch {
            replica_id: replica.to_string(),
            connection_generation: 1,
        };
        let first = session("actor-parity-replica-1");
        let second = session("actor-parity-replica-2");
        let candidates = [first.clone(), second.clone()];
        let aaa = format!("sha256:{}", "a".repeat(64));
        let bbb = format!("sha256:{}", "b".repeat(64));
        assert_eq!(
            pick_owner_candidate(&candidates, &aaa).expect("owner"),
            &second
        );
        assert_eq!(
            pick_owner_candidate(&candidates, &bbb).expect("owner"),
            &first
        );
        assert_eq!(
            pick_owner_candidate(&candidates, &aaa).expect("owner"),
            pick_owner_candidate(&candidates, &aaa).expect("owner")
        );
        assert_eq!(pick_owner_candidate(&[], &aaa), None);
        let three = [first, second.clone(), session("actor-parity-replica-3")];
        assert_eq!(pick_owner_candidate(&three, &bbb).expect("owner"), &second);
    }
}
