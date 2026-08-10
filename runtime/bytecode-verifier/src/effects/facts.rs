use skiff_artifact_model::{CallableMayEffects, PackageCallableId};
use skiff_runtime_linked_bytecode::FunctionIndex;

/// Opaque verifier-owned effect certificate in dense function order.
#[derive(Debug)]
pub struct VerifiedCallableEffects {
    functions: Box<[VerifiedFunctionEffects]>,
}

impl VerifiedCallableEffects {
    pub(super) fn new(functions: Box<[VerifiedFunctionEffects]>) -> Self {
        Self { functions }
    }

    /// Returns the number of dense verified function coordinates.
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    /// Returns the authoritative effect certificate at one dense coordinate.
    pub fn function(&self, function: FunctionIndex) -> Option<&VerifiedFunctionEffects> {
        self.functions
            .get(function.get() as usize)
            .filter(|facts| facts.function == function)
    }
}

/// Authoritative analyzed effects for one exact canonical callable.
#[derive(Debug)]
pub struct VerifiedFunctionEffects {
    function: FunctionIndex,
    canonical_callable: PackageCallableId,
    effects: CallableMayEffects,
    no_pending: bool,
}

impl VerifiedFunctionEffects {
    pub(super) fn new(
        function: FunctionIndex,
        canonical_callable: PackageCallableId,
        effects: CallableMayEffects,
    ) -> Self {
        let no_pending = effects.pending_effect_categories.is_empty();
        Self {
            function,
            canonical_callable,
            effects,
            no_pending,
        }
    }

    /// Returns this certificate's dense function coordinate.
    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    /// Returns the exact canonical callable that owns the analyzed effects.
    pub const fn canonical_callable(&self) -> &PackageCallableId {
        &self.canonical_callable
    }

    /// Returns the authoritative analyzed may-effect lattice value.
    pub const fn effects(&self) -> &CallableMayEffects {
        &self.effects
    }

    /// Returns whether the canonical pending category set is empty.
    pub const fn no_pending(&self) -> bool {
        self.no_pending
    }
}
