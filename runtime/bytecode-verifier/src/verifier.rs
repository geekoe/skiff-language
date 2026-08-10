mod constants;
mod entries;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    ContractOperationId, GatewayAdapterPlan, GatewayEntryIdentity, GatewayEntryKey,
    GatewayEntryProtocolSurface, PackageCallableId,
};
use skiff_runtime_deployment_image::{
    DeploymentOwnerIdentity, DeploymentProgramFacts, ServiceDependencySlot,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedBytecodeCandidate, LinkedCallableSignature, LinkedFunction,
    LinkedGatewayCallableRole,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{
    admission::prove_admission,
    attribution::{
        prove_source_attribution, prove_statement_attribution, VerifiedStatementSchedule,
    },
    concrete_values::prove_types_and_plans,
    control_flow,
    effects::{prove_effect_and_no_pending, VerifiedCallableEffects, VerifiedFunctionEffects},
    VerificationError, VerificationLimits, VerificationLocation, VerificationObligation,
};

pub(super) use constants::prove_and_build_empty_constant_heap;
pub use constants::VerifiedConstantHeap;
pub use entries::{CodeEntryLookupError, VerifiedCodeEntry, VerifiedCodeEntryKind};
#[derive(Debug)]
struct VerificationSeal;

#[derive(Debug)]
struct SealedDeploymentFacts {
    owner: DeploymentOwnerIdentity,
    dependency_slots: Box<[ServiceDependencySlot]>,
    entry_maps: VerifiedEntryMaps,
    constant_heap: VerifiedConstantHeap,
    statement_schedule: VerifiedStatementSchedule,
    callable_effects: VerifiedCallableEffects,
    seal: VerificationSeal,
}

#[derive(Debug)]
struct VerifiedSemanticFacts {
    constant_heap: VerifiedConstantHeap,
    statement_schedule: VerifiedStatementSchedule,
    callable_effects: VerifiedCallableEffects,
}

#[derive(Debug)]
struct VerifiedEntryMaps {
    operations: BTreeMap<ContractOperationId, VerifiedCallableEntryFacts>,
    gateways: BTreeMap<GatewayEntryKey, VerifiedGatewayEntryFacts>,
}

#[derive(Debug)]
struct VerifiedCallableEntryFacts {
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

#[derive(Debug)]
struct VerifiedGatewayCallableFacts {
    _package_callable_id: PackageCallableId,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

#[derive(Debug)]
struct VerifiedGatewayEntryFacts {
    identity: GatewayEntryIdentity,
    _protocol_surface: GatewayEntryProtocolSurface,
    callables: BTreeMap<LinkedGatewayCallableRole, VerifiedGatewayCallableFacts>,
    _adapter_plan: GatewayAdapterPlan,
    _close_adapter_plan: Option<GatewayAdapterPlan>,
}

/// A linked candidate sealed against one exact, opaque deployment hydration.
///
/// Fields are private and there is no `Default`, unchecked/test-support
/// constructor, `From<LinkedBytecodeCandidate>`, mutable candidate accessor,
/// or `DerefMut` implementation. The exact owner is always derived from the
/// consumed hydration; [`verify`] has no caller-supplied owner parameter.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
/// use skiff_runtime_loader::HydratedDeploymentBytecode;
///
/// fn extract_unverified_hydration(
///     image: &VerifiedLinkedBytecodeImage,
/// ) -> &HydratedDeploymentBytecode {
///     &image._hydrated
/// }
/// ```
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
/// use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
///
/// fn bypass(candidate: LinkedBytecodeCandidate) -> VerifiedLinkedBytecodeImage {
///     candidate.into()
/// }
/// ```
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
///
/// let _ = VerifiedLinkedBytecodeImage::default();
/// ```
#[derive(Debug)]
pub struct VerifiedLinkedBytecodeImage {
    _hydrated: HydratedDeploymentBytecode,
    candidate: LinkedBytecodeCandidate,
    owner: DeploymentOwnerIdentity,
    dependency_slots: Box<[ServiceDependencySlot]>,
    entry_maps: VerifiedEntryMaps,
    constant_heap: VerifiedConstantHeap,
    statement_schedule: VerifiedStatementSchedule,
    callable_effects: VerifiedCallableEffects,
    _seal: VerificationSeal,
}

impl VerifiedLinkedBytecodeImage {
    /// Returns the exact deployment owner derived from the hydration.
    pub const fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    /// Returns the complete verified candidate as a shared, read-only view.
    pub const fn candidate(&self) -> &LinkedBytecodeCandidate {
        &self.candidate
    }

    /// Returns the verified concrete functions as a shared, read-only view.
    pub fn functions(&self) -> &[LinkedFunction] {
        self.candidate.functions()
    }

    /// Returns canonical symbolic service facts derived from the hydration.
    pub fn dependency_slots(&self) -> &[ServiceDependencySlot] {
        &self.dependency_slots
    }

    /// Returns the immutable constant heap materialized by this verifier.
    pub const fn constant_heap(&self) -> &VerifiedConstantHeap {
        &self.constant_heap
    }

    /// Returns the verifier-derived immutable semantic charging schedule.
    pub const fn statement_schedule(&self) -> &VerifiedStatementSchedule {
        &self.statement_schedule
    }

    /// Returns authoritative analyzed effects for one dense function.
    pub fn function_effects(&self, function: FunctionIndex) -> Option<&VerifiedFunctionEffects> {
        self.callable_effects.function(function)
    }

    /// Resolves a verified service operation entry while pinning this exact
    /// program allocation.
    pub fn operation_entry(
        self: &Arc<Self>,
        operation: &ContractOperationId,
    ) -> Result<VerifiedCodeEntry, CodeEntryLookupError> {
        let entry = self.entry_maps.operations.get(operation).ok_or_else(|| {
            CodeEntryLookupError::OperationNotFound {
                contract_operation_id: operation.clone(),
            }
        })?;
        let function = entry.function;
        let signature = entry.signature.clone();

        Ok(VerifiedCodeEntry {
            program: Arc::clone(self),
            kind: VerifiedCodeEntryKind::Operation {
                contract_operation_id: operation.clone(),
            },
            function,
            signature,
        })
    }

    /// Resolves one typed callable role by its owner-local gateway key and pins
    /// this program allocation. The returned kind carries the identity proved
    /// from the hydration; callers never supply that fact.
    pub fn gateway_entry(
        self: &Arc<Self>,
        key: &GatewayEntryKey,
        role: LinkedGatewayCallableRole,
    ) -> Result<VerifiedCodeEntry, CodeEntryLookupError> {
        let entry = self.entry_maps.gateways.get(key).ok_or_else(|| {
            CodeEntryLookupError::GatewayNotFound {
                gateway_entry_key: key.clone(),
            }
        })?;
        let callable = entry.callables.get(&role).ok_or_else(|| {
            CodeEntryLookupError::GatewayCallableNotFound {
                gateway_entry_key: key.clone(),
                gateway_entry_identity: entry.identity.clone(),
                role,
            }
        })?;
        let function = callable.function;
        let signature = callable.signature.clone();

        Ok(VerifiedCodeEntry {
            program: Arc::clone(self),
            kind: VerifiedCodeEntryKind::Gateway {
                gateway_entry_key: key.clone(),
                gateway_entry_identity: entry.identity.clone(),
                role,
            },
            function,
            signature,
        })
    }
}

impl DeploymentProgramFacts for VerifiedLinkedBytecodeImage {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    fn dependency_slots(&self) -> &[ServiceDependencySlot] {
        &self.dependency_slots
    }
}

/// Independently verifies and seals one exact deployment hydration and linked
/// candidate as a single immutable program.
///
/// Both inputs are consumed so a successful result owns the exact facts that
/// were cross-checked. Admission independently proves bounded exact hydration,
/// artifact and candidate correspondence before every supported semantic
/// proof family completes. Unsupported slices fail closed before seal minting.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::{verify, VerificationLimits};
/// use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
///
/// fn candidate_only_bypass(
///     candidate: LinkedBytecodeCandidate,
///     limits: &VerificationLimits,
/// ) {
///     let _ = verify(candidate, limits);
/// }
/// ```
pub fn verify(
    hydrated: HydratedDeploymentBytecode,
    candidate: LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<VerifiedLinkedBytecodeImage, VerificationError> {
    let facts = establish_verification_seal(&hydrated, &candidate, limits)?;
    Ok(VerifiedLinkedBytecodeImage {
        _hydrated: hydrated,
        candidate,
        owner: facts.owner,
        dependency_slots: facts.dependency_slots,
        entry_maps: facts.entry_maps,
        constant_heap: facts.constant_heap,
        statement_schedule: facts.statement_schedule,
        callable_effects: facts.callable_effects,
        _seal: facts.seal,
    })
}

fn establish_verification_seal(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<SealedDeploymentFacts, VerificationError> {
    let admission = prove_admission(hydrated, candidate, limits)?;
    let semantics = prove_hydrated_candidate_semantics(hydrated, candidate, &admission, limits)?;
    let entry_maps = distill_verified_entry_maps(candidate)?;

    Ok(SealedDeploymentFacts {
        owner: DeploymentOwnerIdentity::new(hydrated.reference().clone()),
        dependency_slots: dependency_slots_from_hydration(hydrated)?,
        entry_maps,
        constant_heap: semantics.constant_heap,
        statement_schedule: semantics.statement_schedule,
        callable_effects: semantics.callable_effects,
        seal: VerificationSeal,
    })
}

fn prove_hydrated_candidate_semantics(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    admission: &crate::admission::AdmissionFacts,
    limits: &VerificationLimits,
) -> Result<VerifiedSemanticFacts, VerificationError> {
    let concrete_values = prove_types_and_plans(hydrated, candidate, limits)?;
    let control_flow =
        control_flow::prove_control_flow_and_stack(hydrated, candidate, &concrete_values, limits)?;
    let source = prove_source_attribution(candidate)?;
    let statement_schedule = prove_statement_attribution(
        candidate,
        admission.statement_binding(),
        &source,
        control_flow.control_flow(),
        limits,
    )?;
    let callable_effects = prove_effect_and_no_pending(
        admission.effect_binding(),
        &control_flow,
        &statement_schedule,
    )?;
    let constant_heap = prove_and_build_empty_constant_heap(hydrated, candidate)?;
    Ok(VerifiedSemanticFacts {
        constant_heap,
        statement_schedule,
        callable_effects,
    })
}

#[cfg(test)]
pub(super) fn prove_statement_schedule_for_test(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<VerifiedStatementSchedule, VerificationError> {
    let admission = prove_admission(hydrated, candidate, limits)?;
    let concrete_values = prove_types_and_plans(hydrated, candidate, limits)?;
    let control_flow =
        control_flow::prove_control_flow_and_stack(hydrated, candidate, &concrete_values, limits)?;
    let source = prove_source_attribution(candidate)?;
    prove_statement_attribution(
        candidate,
        admission.statement_binding(),
        &source,
        control_flow.control_flow(),
        limits,
    )
}

/// A candidate alone can never enter P2 because concrete type resolution
/// requires the exact admitted hydration. This narrow test seam preserves the
/// fail-closed candidate-only invariant without becoming a verification path.
#[cfg(test)]
pub(super) fn prove_candidate_semantics(
    _candidate: &LinkedBytecodeCandidate,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ConcreteTypeAndShape,
        location: VerificationLocation::Image,
    })
}

fn distill_verified_entry_maps(
    candidate: &LinkedBytecodeCandidate,
) -> Result<VerifiedEntryMaps, VerificationError> {
    let mut operations = BTreeMap::new();
    for entry in candidate.operation_entries() {
        let operation = entry.contract_operation_id().clone();
        let facts = VerifiedCallableEntryFacts {
            function: entry.function(),
            signature: entry.signature().clone(),
        };
        if operations.insert(operation.clone(), facts).is_some() {
            return Err(VerificationError::SemanticViolation {
                obligation: VerificationObligation::ExactHydrationBinding,
                location: VerificationLocation::Image,
                detail: format!(
                    "verified operation entry {operation} appears more than once after proof"
                ),
            });
        }
    }

    let mut gateways = BTreeMap::new();
    for entry in candidate.gateway_entries() {
        let key = entry.gateway_entry_key().clone();
        let mut callables = BTreeMap::new();
        for callable in entry.callables() {
            let role = callable.role();
            let facts = VerifiedGatewayCallableFacts {
                _package_callable_id: callable.package_callable_id().clone(),
                function: callable.function(),
                signature: callable.signature().clone(),
            };
            if callables.insert(role, facts).is_some() {
                return Err(VerificationError::SemanticViolation {
                    obligation: VerificationObligation::ExactHydrationBinding,
                    location: VerificationLocation::Image,
                    detail: format!(
                        "verified gateway entry {key} role {role:?} appears more than once after proof"
                    ),
                });
            }
        }
        let facts = VerifiedGatewayEntryFacts {
            identity: entry.gateway_entry_identity().clone(),
            _protocol_surface: entry.protocol_surface().clone(),
            callables,
            _adapter_plan: entry.adapter_plan().clone(),
            _close_adapter_plan: entry.close_adapter_plan().cloned(),
        };
        if gateways.insert(key.clone(), facts).is_some() {
            return Err(VerificationError::SemanticViolation {
                obligation: VerificationObligation::ExactHydrationBinding,
                location: VerificationLocation::Image,
                detail: format!("verified gateway entry {key} appears more than once after proof"),
            });
        }
    }

    Ok(VerifiedEntryMaps {
        operations,
        gateways,
    })
}

fn dependency_slots_from_hydration(
    hydrated: &HydratedDeploymentBytecode,
) -> Result<Box<[ServiceDependencySlot]>, VerificationError> {
    hydrated
        .service_dependencies()
        .values()
        .map(|dependency| {
            ServiceDependencySlot::try_new(
                dependency.key().clone(),
                dependency.contract().clone(),
                dependency.used_operations().iter().cloned(),
            )
            .map_err(|error| VerificationError::SemanticViolation {
                obligation: VerificationObligation::ExactHydrationBinding,
                location: VerificationLocation::Image,
                detail: error.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}
