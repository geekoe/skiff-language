// Spawned actor methods are detached from the caller and receive their own
// execution deadline. Keep the owner lease longer than that deadline so the
// lease sweeper cannot fence a healthy owner while the invocation is active.
export const SPAWNED_ACTOR_METHOD_DEADLINE_MS = 300_000;
export const ACTOR_OWNER_LEASE_TTL_MS = 330_000;
