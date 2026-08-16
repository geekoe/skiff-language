//! Heap-neutral memory lease primitives shared by the request ledger and child
//! heap carriers.
//!
//! The lease itself deliberately contains no allocator, global registry or
//! accounting state. It is an affine handle to a request-scoped
//! [`MemoryLeaseHost`]; the request ledger owns the exact reserve/commit/
//! release accounting.

use std::{fmt, num::NonZeroU64, sync::Arc};

/// One request-scoped memory lease identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryLeaseToken(NonZeroU64);

impl MemoryLeaseToken {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Exact release authority behind a [`MemoryLease`].
///
/// A host must release each token at most once and must keep the total amount
/// in its committed accounting. Implementations may use this to reject
/// forged tokens by keeping a token table, but they must not report an error
/// through this synchronous trait; a correctly minted lease is infallible.
pub trait MemoryLeaseHost: Send + Sync + 'static {
    fn release_lease(&self, token: MemoryLeaseToken, amount: usize);
}

/// One committed request-memory owner.
///
/// `MemoryLease` is intentionally not `Clone`. It releases exactly one
/// committed amount when consumed by [`MemoryLease::release`] or dropped.
#[must_use = "a memory lease must be released exactly once"]
pub struct MemoryLease {
    host: Arc<dyn MemoryLeaseHost>,
    token: MemoryLeaseToken,
    amount: usize,
    released: bool,
}

impl MemoryLease {
    /// Binds one host-minted token to an affine lease.
    ///
    /// Callers must only use a token and amount returned by the same host's
    /// reservation/commit operation. This constructor is public so the
    /// request ledger can produce leases without model-owned allocator logic.
    pub fn new(host: Arc<dyn MemoryLeaseHost>, token: MemoryLeaseToken, amount: usize) -> Self {
        Self {
            host,
            token,
            amount,
            released: false,
        }
    }

    pub const fn token(&self) -> MemoryLeaseToken {
        self.token
    }

    pub const fn amount(&self) -> usize {
        self.amount
    }

    /// Replaces the committed amount tracked by this lease.
    ///
    /// This is host-owned: the owning ledger must already have applied the
    /// corresponding reserve/commit/release delta before calling this setter.
    /// It exists so a concrete heap can keep one affine lease in step with its
    /// current live allocation while the heap remains reachable.
    pub fn set_amount(&mut self, amount: usize) {
        self.amount = amount;
    }

    /// Releases the committed amount exactly once.
    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.host.release_lease(self.token, self.amount);
    }
}

impl Drop for MemoryLease {
    fn drop(&mut self) {
        self.release_inner();
    }
}

impl fmt::Debug for MemoryLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryLease")
            .field("token", &self.token)
            .field("amount", &self.amount)
            .field("released", &self.released)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroU64,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use super::{MemoryLease, MemoryLeaseHost, MemoryLeaseToken};

    #[derive(Default)]
    struct CountingHost(AtomicUsize);

    impl MemoryLeaseHost for CountingHost {
        fn release_lease(&self, _token: MemoryLeaseToken, amount: usize) {
            self.0.fetch_add(amount, Ordering::SeqCst);
        }
    }

    #[test]
    fn memory_lease_releases_exactly_once_on_drop_and_explicit_release() {
        let host = Arc::new(CountingHost::default());
        let token = MemoryLeaseToken::new(NonZeroU64::new(7).unwrap());

        let lease = MemoryLease::new(Arc::clone(&host) as Arc<dyn MemoryLeaseHost>, token, 3);
        drop(lease);
        let lease = MemoryLease::new(Arc::clone(&host) as Arc<dyn MemoryLeaseHost>, token, 5);
        lease.release();

        assert_eq!(host.0.load(Ordering::SeqCst), 8);
    }
}
