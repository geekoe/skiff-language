mod closure;
mod constants;
mod functions;
mod relocations;
mod tables;
mod targets;

use std::collections::BTreeMap;

use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedBytecodeCandidate, LinkedBytecodeCandidateParts, LinkedExactLocalTarget,
    SpecializationKey,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::bytecode::{
    limits::LinkLimitTracker, types::TypeLinker, BytecodeLinkError, BytecodeLinkLimit,
    BytecodeLinkLocation, BytecodeLinkObligation, LinkLimits,
};

pub(super) struct DeploymentLinker<'a> {
    deployment: &'a HydratedDeploymentBytecode,
    limits: &'a LinkLimits,
    tracker: LinkLimitTracker<'a>,
}

impl<'a> DeploymentLinker<'a> {
    pub(super) fn new(deployment: &'a HydratedDeploymentBytecode, limits: &'a LinkLimits) -> Self {
        Self {
            deployment,
            limits,
            tracker: LinkLimitTracker::new(limits),
        }
    }

    pub(super) fn link(mut self) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> {
        let deployment_location = self.deployment_location();
        self.validate_exact_package_closure()?;
        self.reject_unsupported_global_authorities()?;
        let packages = self.link_package_provenance()?;
        let roots = self.canonical_roots()?;
        let keys = self.discover_closure(roots)?;
        let function_indices = canonical_function_indices(&keys, deployment_location.clone())?;

        let mut type_linker = TypeLinker::new(self.limits);
        let frames = keys
            .iter()
            .map(|key| self.link_frame(key, &mut type_linker))
            .collect::<Result<Vec<_>, _>>()?;
        let functions = keys
            .iter()
            .map(|key| {
                let index = function_indices.get(key).copied().ok_or_else(|| {
                    unsatisfied(
                        BytecodeLinkObligation::ConcreteSpecialization,
                        deployment_location.clone(),
                        "canonical specialization has no final function index".to_string(),
                    )
                })?;
                self.link_function(key, index, &function_indices, &frames, &mut type_linker)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let operation_entries = self.link_operation_entries(&function_indices, &functions)?;
        let gateway_entries = self.link_gateway_entries(&function_indices, &functions)?;
        let exact_local_targets = keys
            .iter()
            .map(|key| {
                function_indices
                    .get(key)
                    .copied()
                    .map(|index| LinkedExactLocalTarget::new(key.clone(), index))
                    .ok_or_else(|| {
                        unsatisfied(
                            BytecodeLinkObligation::ConcreteSpecialization,
                            deployment_location.clone(),
                            "canonical specialization has no exact-local index".to_string(),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let types = type_linker.finish(deployment_location.clone())?;

        for count in [
            packages.len(),
            functions.len(),
            operation_entries.len(),
            gateway_entries.len(),
            exact_local_targets.len(),
            types.len(),
        ] {
            self.tracker
                .add_image_table(count as u64, deployment_location.clone())?;
        }

        LinkedBytecodeCandidate::try_from_parts(LinkedBytecodeCandidateParts {
            packages,
            functions,
            operation_entries,
            gateway_entries,
            exact_local_targets,
            service_operations: Vec::new(),
            actor_creates: Vec::new(),
            actor_methods: Vec::new(),
            interface_tables: Vec::new(),
            synthetic_callbacks: Vec::new(),
            callback_capture_layouts: Vec::new(),
            host_effect_adapters: Vec::new(),
            intrinsics: Vec::new(),
            types,
            shapes: Vec::new(),
            constants: Vec::new(),
            constant_roots: Vec::new(),
            frozen_constant_nodes: Vec::new(),
            resume_sites: Vec::new(),
            writable_paths: Vec::new(),
        })
        .map_err(|error| {
            unsatisfied(
                BytecodeLinkObligation::CandidateAssembly,
                deployment_location,
                error.to_string(),
            )
        })
    }
}

fn canonical_function_indices(
    keys: &[SpecializationKey],
    location: BytecodeLinkLocation,
) -> Result<BTreeMap<SpecializationKey, FunctionIndex>, BytecodeLinkError> {
    keys.iter()
        .cloned()
        .enumerate()
        .map(|(index, key)| {
            let index = u32::try_from(index).map_err(|_| BytecodeLinkError::LimitExceeded {
                limit: BytecodeLinkLimit::Specializations,
                actual: keys.len() as u64,
                max: u32::MAX as u64,
                location: location.clone(),
            })?;
            Ok((key, FunctionIndex::new(index)))
        })
        .collect()
}

fn unsatisfied(
    obligation: BytecodeLinkObligation,
    location: BytecodeLinkLocation,
    detail: String,
) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation,
        location,
        detail,
    }
}
