mod closure;
mod constants;
pub(super) mod dispatch;
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
        let mut type_linker = TypeLinker::new(self.deployment, self.limits);
        let mut roots = self.canonical_roots()?;
        self.extend_target_roots(&mut roots, &mut type_linker)?;
        let keys = self.discover_closure(roots)?;
        let function_indices = canonical_function_indices(&keys, deployment_location.clone())?;
        type_linker.set_function_indices(&function_indices);

        let constant_tables = self.link_constant_tables(&mut type_linker)?;
        let frames = keys
            .iter()
            .map(|key| self.link_frame(key, &mut type_linker))
            .collect::<Result<Vec<_>, _>>()?;
        let dispatch_tables = self.link_dispatch_tables(&function_indices, &frames, &mut type_linker)?;
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
                self.link_function(
                    key,
                    index,
                    &function_indices,
                    &frames,
                    &constant_tables,
                    &dispatch_tables,
                    &mut type_linker,
                )
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
        let pool_tables = type_linker.finish(deployment_location.clone())?;
        let (constants, constant_roots, frozen_constant_nodes) = constant_tables.into_parts();

        for count in [
            packages.len(),
            functions.len(),
            operation_entries.len(),
            gateway_entries.len(),
            exact_local_targets.len(),
            pool_tables.types.len(),
            pool_tables.shapes.len(),
            pool_tables.writable_paths.len(),
            pool_tables.callback_capture_layouts.len(),
            pool_tables.resume_sites.len(),
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
            service_operations: dispatch_tables.service_operations,
            actor_creates: dispatch_tables.actor_creates,
            actor_methods: dispatch_tables.actor_methods,
            interface_tables: dispatch_tables.interface_tables,
            synthetic_callbacks: dispatch_tables.synthetic_callbacks,
            callback_capture_layouts: pool_tables.callback_capture_layouts,
            host_effect_adapters: dispatch_tables.host_effect_adapters,
            intrinsics: dispatch_tables.intrinsics,
            types: pool_tables.types,
            shapes: pool_tables.shapes,
            constants,
            constant_roots,
            frozen_constant_nodes,
            resume_sites: pool_tables.resume_sites,
            writable_paths: pool_tables.writable_paths,
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
