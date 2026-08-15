//! A6 Actor child leaf.
//!
//! The X6 mux owns registration and the K6 scheduler owns the shared Actor
//! arena/segment lifecycle. This leaf only carries the exact Actor identity
//! checks and fail-closed composition requirements that A6 can prove without
//! introducing a second owner registry or child loop.

/// Request-scoped Actor child composition supplied by the X6 composition.
#[derive(Clone, Default)]
pub struct BytecodeActorChildComposition {
    /// Exact deployment build pinned by the actor owner fence.
    pub exact_build: Option<String>,
    /// K6-owned shared arena lease root; absent keeps every Actor child
    /// fail-closed until the kernel seam is joined.
    pub arena_lease_root: Option<String>,
}

impl BytecodeActorChildComposition {
    pub fn is_available(&self) -> bool {
        self.exact_build.is_some() && self.arena_lease_root.is_some()
    }

    pub fn require_exact_build(&self, expected: &str) -> Result<(), ActorChildError> {
        match self.exact_build.as_deref() {
            Some(actual) if actual == expected => Ok(()),
            Some(actual) => Err(ActorChildError::BuildMismatch {
                expected: expected.to_string(),
                actual: actual.to_string(),
            }),
            None => Err(ActorChildError::MissingExactBuild),
        }
    }

    pub fn require_arena_lease(&self) -> Result<(), ActorChildError> {
        if self.arena_lease_root.is_some() {
            Ok(())
        } else {
            Err(ActorChildError::MissingArenaLease)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActorChildError {
    #[error("actor child exact deployment build is missing")]
    MissingExactBuild,
    #[error("actor child arena lease root is missing")]
    MissingArenaLease,
    #[error("actor child build mismatch: expected {expected}, actual {actual}")]
    BuildMismatch { expected: String, actual: String },
    #[error("actor child shared arena is not available")]
    ArenaUnavailable,
}

/// Fail-closed reminder for the central join requirements. The actor leaf
/// cannot reconstruct exact build, arena or method-table facts from request
/// metadata.
#[cfg(test)]
pub(crate) fn actor_child_required_fact() -> &'static str {
    "F6/K6/X6 must join exact actor build/image, shared arena/segment lease and \
     request child registration before Actor get/create/method is reachable"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_child_composition_requires_exact_build_and_arena_lease() {
        let composition = BytecodeActorChildComposition::default();
        assert!(!composition.is_available());
        assert_eq!(
            composition.require_exact_build("build-1"),
            Err(ActorChildError::MissingExactBuild)
        );
        assert_eq!(
            composition.require_arena_lease(),
            Err(ActorChildError::MissingArenaLease)
        );
    }

    #[test]
    fn actor_child_composition_rejects_cross_build_before_execution() {
        let composition = BytecodeActorChildComposition {
            exact_build: Some("build-1".to_string()),
            arena_lease_root: Some("arena-1".to_string()),
        };
        assert!(composition.is_available());
        assert!(composition.require_exact_build("build-1").is_ok());
        assert_eq!(
            composition.require_exact_build("build-2"),
            Err(ActorChildError::BuildMismatch {
                expected: "build-2".to_string(),
                actual: "build-1".to_string(),
            })
        );
    }

    #[test]
    fn actor_child_required_fact_names_central_owners() {
        assert!(actor_child_required_fact().contains("F6/K6/X6"));
    }
}
