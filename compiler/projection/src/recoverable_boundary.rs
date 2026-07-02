use std::collections::{BTreeMap, BTreeSet};

use crate::contract::{validate_static_type_ref_boundary_policy, BoundaryKind};
use crate::error::ProjectionError;
use skiff_artifact_model::{
    type_ref_abi_key, DbMetadataIr, FieldPathIr, FileIrUnit, InterfaceDeclIr,
    InterfaceInstantiationRef, InterfaceOperationIr, PackageRefIr, PackageSymbolRef, PackageUnit,
    RecoverableArtifactMetadata, RecoverableBoundaryContext, RecoverableBoundaryKind,
    RecoverableBoundaryPlan, RecoverableCustomRestorePlan, RecoverableCustomRestorePlanRef,
    RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot, RecoverableFieldIdentityFact,
    RecoverableFieldIdentityRef, RecoverableInterfaceMethodIdentityFact,
    RecoverableInterfaceMethodIdentityRef, RecoverableInterfaceProjectionIdentityFact,
    RecoverableInterfaceProjectionIdentityRef, RecoverableNativeAdapterPlan,
    RecoverableNativeAdapterPlanRef, RecoverableStorageLane, RecoverableStorageLanePlan,
    RecoverableStorageLaneRef, RecoverableTrustBoundary, RecoverableTypeIdentityFact,
    RecoverableTypeIdentityRef, RecoverableUnionBranchIdentityFact,
    RecoverableUnionBranchIdentityRef, SpawnTargetIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
};

#[derive(Clone, Debug)]
pub struct RecoverablePackageTypeSource {
    pub package_id: String,
    pub dependency_refs: Vec<String>,
    pub unit: PackageUnit,
    pub file_ir_units: Vec<FileIrUnit>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RecoverableInputs<'a> {
    pub package_sources: &'a [RecoverablePackageTypeSource],
    pub custom_restore_plans: &'a [RecoverableCustomRestorePlan],
    pub native_adapter_plans: &'a [RecoverableNativeAdapterPlan],
}

pub fn recoverable_metadata_for_service_artifacts(
    service_id: &str,
    file_ir_units: &[FileIrUnit],
    db_metadata: &[DbMetadataIr],
    spawn_targets: &[SpawnTargetIr],
    inputs: RecoverableInputs<'_>,
) -> Result<RecoverableArtifactMetadata, ProjectionError> {
    let mut builder =
        RecoverableMetadataBuilder::new(Some(service_id.to_string()), file_ir_units, inputs);
    for db in db_metadata {
        builder.add_db_metadata(db)?;
    }
    for target in spawn_targets {
        builder.add_spawn_target(target)?;
    }
    builder.validate_declared_recoverable_plans()?;
    let mut metadata = builder.finish();
    metadata.capabilities.flags.insert(
        "recoverableCompilerBoundaryPlanV1".to_string(),
        skiff_artifact_model::RecoverableCapabilityFlag {
            enabled: true,
            revision: Some(1),
        },
    );
    skiff_artifact_model::validate_recoverable_artifact_metadata(&metadata).map_err(|error| {
        ProjectionError::ContractValidation {
            message: format!("recoverable artifact metadata validation failed: {error}"),
        }
    })?;
    Ok(metadata)
}

pub fn validate_recoverable_metadata_type_policy(
    metadata: &RecoverableArtifactMetadata,
    file_ir_units: &[FileIrUnit],
) -> Result<(), ProjectionError> {
    validate_recoverable_metadata_type_policy_with_packages(metadata, file_ir_units, &[])
}

pub fn validate_recoverable_metadata_type_policy_with_packages(
    metadata: &RecoverableArtifactMetadata,
    file_ir_units: &[FileIrUnit],
    package_sources: &[RecoverablePackageTypeSource],
) -> Result<(), ProjectionError> {
    let native_adapter_plans = metadata
        .native_adapter_plans
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let custom_restore_plans = metadata
        .custom_restore_plans
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut builder = RecoverableMetadataBuilder::new(
        None,
        file_ir_units,
        RecoverableInputs {
            package_sources,
            custom_restore_plans: &custom_restore_plans,
            native_adapter_plans: &native_adapter_plans,
        },
    );
    builder.metadata = metadata.clone();
    for (key, plan) in &metadata.custom_restore_plans {
        builder.validate_expected_type_plan_closure(
            metadata,
            &plan.durable_state_type_plan,
            BoundaryKind::RecoverablePayload,
            &format!("custom restore plan {key} durable state type"),
        )?;
    }
    for (key, plan) in &metadata.native_adapter_plans {
        builder.validate_expected_type_plan_closure(
            metadata,
            &plan.durable_state_type_plan,
            BoundaryKind::RecoverablePayload,
            &format!("native adapter plan {key} durable state type"),
        )?;
    }
    Ok(())
}

pub fn recoverable_boundary_plan_for_type(
    key: impl Into<String>,
    module_path: &str,
    file_ir_units: &[FileIrUnit],
    ty: &TypeRefIr,
    boundary_kind: RecoverableBoundaryKind,
    trust_boundary: RecoverableTrustBoundary,
    explicit_recoverable_slot: bool,
    inputs: RecoverableInputs<'_>,
) -> Result<(String, RecoverableBoundaryPlan), ProjectionError> {
    let mut builder = RecoverableMetadataBuilder::new(None, file_ir_units, inputs);
    let context = RecoverableBoundaryContext {
        boundary_kind,
        trust_boundary,
        origin_service: None,
        target_service: None,
        explicit_recoverable_slot,
    };
    let key = key.into();
    let plan = builder.boundary_plan(module_path, &context, ty, None)?;
    Ok((key, plan))
}

struct RecoverableMetadataBuilder {
    origin_service: Option<String>,
    file_ir_units: Vec<FileIrUnit>,
    package_sources: Vec<RecoverablePackageTypeSource>,
    package_db_modules: BTreeMap<String, String>,
    ambiguous_package_db_modules: BTreeSet<String>,
    metadata: RecoverableArtifactMetadata,
}

impl RecoverableMetadataBuilder {
    fn new(
        origin_service: Option<String>,
        file_ir_units: &[FileIrUnit],
        inputs: RecoverableInputs<'_>,
    ) -> Self {
        let mut metadata = RecoverableArtifactMetadata::default();
        for plan in inputs.custom_restore_plans {
            metadata
                .custom_restore_plans
                .insert(custom_restore_plan_key(plan), plan.clone());
        }
        for plan in inputs.native_adapter_plans {
            metadata
                .native_adapter_plans
                .insert(native_adapter_plan_key(plan), plan.clone());
        }
        Self {
            origin_service,
            file_ir_units: file_ir_units.to_vec(),
            package_sources: inputs.package_sources.to_vec(),
            package_db_modules: BTreeMap::new(),
            ambiguous_package_db_modules: BTreeSet::new(),
            metadata,
        }
    }

    fn finish(self) -> RecoverableArtifactMetadata {
        self.metadata
    }

    fn validate_declared_recoverable_plans(&self) -> Result<(), ProjectionError> {
        for (key, plan) in &self.metadata.custom_restore_plans {
            self.validate_expected_type_plan_closure(
                &self.metadata,
                &plan.durable_state_type_plan,
                BoundaryKind::RecoverablePayload,
                &format!("custom restore plan {key} durable state type"),
            )?;
        }
        for (key, plan) in &self.metadata.native_adapter_plans {
            self.validate_expected_type_plan_closure(
                &self.metadata,
                &plan.durable_state_type_plan,
                BoundaryKind::RecoverablePayload,
                &format!("native adapter plan {key} durable state type"),
            )?;
        }
        Ok(())
    }

    fn add_db_metadata(&mut self, db: &DbMetadataIr) -> Result<(), ProjectionError> {
        self.record_package_db_module(db);
        for field in &db.fields {
            self.validate_type_ref_closure(
                &db.module_path,
                &field.ty,
                BoundaryKind::DbPayload,
                &format!(
                    "db field {}.{} type cannot cross recoverable DB payload boundary",
                    db.type_name, field.name
                ),
            )?;
            let lane = db_storage_lane_for_type(&field.ty);
            let lane_ref = db_field_lane_ref(db, &field.name);
            let expected_type = self.expected_type_plan(&db.module_path, &field.ty);
            self.metadata.storage_lanes.insert(
                lane_ref.0.clone(),
                RecoverableStorageLanePlan {
                    lane,
                    expected_type: Some(expected_type.clone()),
                    schema_projection_ref: (lane == RecoverableStorageLane::SchemaProjectable)
                        .then(|| format!("db:{}.{}", db.type_name, field.name)),
                    envelope_slot_ref: (lane == RecoverableStorageLane::RecoverableEnvelope)
                        .then(|| format!("db:{}.{}", db.type_name, field.name)),
                },
            );
            let context = RecoverableBoundaryContext {
                boundary_kind: RecoverableBoundaryKind::DbPayload,
                trust_boundary: RecoverableTrustBoundary::OwnerInternal,
                origin_service: self.origin_service.clone(),
                target_service: None,
                explicit_recoverable_slot: false,
            };
            let boundary_key = format!("db:{}:field:{}", db.type_name, field.name);
            let runtime_carrier_check_required = expected_type.runtime_carrier_check_required;
            let custom_restore_plan_ref =
                self.first_custom_restore_plan_ref(&db.module_path, &field.ty);
            let native_adapter_plan_ref =
                self.first_native_adapter_plan_ref(&db.module_path, &field.ty);
            self.metadata.boundary_plans.insert(
                boundary_key,
                RecoverableBoundaryPlan {
                    context,
                    expected_type,
                    runtime_carrier_check_required,
                    storage_lane_ref: Some(lane_ref),
                    custom_restore_plan_ref,
                    native_adapter_plan_ref,
                },
            );
        }
        Ok(())
    }

    fn record_package_db_module(&mut self, db: &DbMetadataIr) {
        if db.source_role != "package" {
            return;
        }
        let Some(package_id) = &db.package_id else {
            return;
        };
        match self.package_db_modules.get(&db.module_path) {
            Some(existing) if existing != package_id => {
                self.ambiguous_package_db_modules
                    .insert(db.module_path.clone());
            }
            Some(_) => {}
            None => {
                self.package_db_modules
                    .insert(db.module_path.clone(), package_id.clone());
            }
        }
    }

    fn add_spawn_target(&mut self, target: &SpawnTargetIr) -> Result<(), ProjectionError> {
        for (index, ty) in target.param_types.iter().enumerate() {
            let context = RecoverableBoundaryContext {
                boundary_kind: RecoverableBoundaryKind::SpawnPayload,
                trust_boundary: RecoverableTrustBoundary::OwnerInternal,
                origin_service: self.origin_service.clone(),
                target_service: self.origin_service.clone(),
                explicit_recoverable_slot: true,
            };
            let module_path = self
                .module_path_for_spawn_target(target)
                .unwrap_or_else(|| target.executable_target.file_ref.module_path.as_str())
                .to_string();
            let plan = self.boundary_plan(&module_path, &context, ty, None)?;
            self.metadata.boundary_plans.insert(
                format!("spawn:{}:param:{index}", target.target_identity),
                plan,
            );
        }
        Ok(())
    }

    fn boundary_plan(
        &mut self,
        module_path: &str,
        context: &RecoverableBoundaryContext,
        ty: &TypeRefIr,
        storage_lane_ref: Option<RecoverableStorageLaneRef>,
    ) -> Result<RecoverableBoundaryPlan, ProjectionError> {
        if context.trust_boundary != RecoverableTrustBoundary::OwnerInternal
            && recoverable_behavior_nodes(ty)
        {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "recoverable {} boundary with {:?} trust cannot contain nominal/custom/native/interface behavior nodes",
                    recoverable_boundary_kind_label(context.boundary_kind),
                    context.trust_boundary
                ),
            });
        }
        let boundary_kind = recoverable_closure_boundary_kind(context);
        self.validate_type_ref_closure(
            module_path,
            ty,
            boundary_kind,
            &format!(
                "type cannot cross recoverable {} boundary",
                recoverable_boundary_kind_label(context.boundary_kind)
            ),
        )?;
        let expected_type = self.expected_type_plan(module_path, ty);
        let runtime_carrier_check_required = expected_type.runtime_carrier_check_required;
        let custom_restore_plan_ref = self.first_custom_restore_plan_ref(module_path, ty);
        let native_adapter_plan_ref = self.first_native_adapter_plan_ref(module_path, ty);
        Ok(RecoverableBoundaryPlan {
            context: context.clone(),
            expected_type,
            runtime_carrier_check_required,
            storage_lane_ref,
            custom_restore_plan_ref,
            native_adapter_plan_ref,
        })
    }

    fn expected_type_plan(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> RecoverableExpectedTypePlan {
        let mut plan = RecoverableExpectedTypePlan {
            root: RecoverableExpectedTypeRoot::TypeRef { ty: ty.clone() },
            root_type_identity_ref: None,
            runtime_carrier_check_required: self.contains_any_interface_closure(module_path, ty),
            interface_projection_refs: Vec::new(),
            interface_method_refs: Vec::new(),
            field_refs: Vec::new(),
            union_branch_refs: Vec::new(),
        };
        self.collect_type_plan_identities(module_path, ty, None, &mut plan);
        plan
    }

    fn collect_type_plan_identities(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        owner: Option<&RecoverableTypeIdentityRef>,
        plan: &mut RecoverableExpectedTypePlan,
    ) {
        match ty {
            TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::PackageSymbol { .. } => {
                let resolved = self
                    .type_decl_for_type_ref_with_module(module_path, ty)
                    .map(|(resolved_module, decl)| (resolved_module.to_string(), decl.clone()));
                let identity_ref =
                    self.nominal_type_identity_ref(module_path, ty, resolved.as_ref());
                self.metadata
                    .identity_tables
                    .types
                    .entry(identity_ref.0.clone())
                    .or_insert(RecoverableTypeIdentityFact {
                        ty: ty.clone(),
                        abi_type_id: Some(identity_ref.0.clone()),
                        contract_revision: None,
                    });
                if plan.root_type_identity_ref.is_none() {
                    plan.root_type_identity_ref = Some(identity_ref.clone());
                }
                if self
                    .custom_restore_plan_for_identity(&identity_ref)
                    .is_some()
                {
                    return;
                }
                if let Some((resolved_module, decl)) = resolved {
                    self.collect_descriptor_identities(
                        &resolved_module,
                        &identity_ref,
                        &decl.descriptor,
                        plan,
                    );
                }
            }
            TypeRefIr::Record { fields } => {
                for (name, field_ty) in fields {
                    if let Some(owner) = owner {
                        let field_ref =
                            RecoverableFieldIdentityRef(format!("field:{}:{}", owner.0, name));
                        self.metadata
                            .identity_tables
                            .fields
                            .entry(field_ref.0.clone())
                            .or_insert(RecoverableFieldIdentityFact {
                                owner_type_ref: owner.clone(),
                                field_name: name.clone(),
                                field_type: Some(field_ty.clone()),
                                field_abi_id: Some(format!(
                                    "{}#{name}",
                                    type_ref_abi_key(field_ty)
                                )),
                            });
                        push_unique(&mut plan.field_refs, field_ref);
                    }
                    self.collect_type_plan_identities(module_path, field_ty, owner, plan);
                }
            }
            TypeRefIr::Union { items } => {
                let union_ref = structural_type_identity_ref(ty);
                for (index, item) in items.iter().enumerate() {
                    let branch_ref =
                        RecoverableUnionBranchIdentityRef(format!("union:{}:{index}", union_ref.0));
                    self.metadata
                        .identity_tables
                        .union_branches
                        .entry(branch_ref.0.clone())
                        .or_insert(RecoverableUnionBranchIdentityFact {
                            union_type_ref: union_ref.clone(),
                            branch_index: index as u32,
                            branch_type: item.clone(),
                            branch_abi_id: Some(type_ref_abi_key(item)),
                        });
                    push_unique(&mut plan.union_branch_refs, branch_ref);
                    self.collect_type_plan_identities(module_path, item, owner, plan);
                }
            }
            TypeRefIr::AnyInterface { interface } => {
                let projection_ref = RecoverableInterfaceProjectionIdentityRef(format!(
                    "interface:{}",
                    interface.interface_abi_id
                ));
                self.metadata
                    .identity_tables
                    .interface_projections
                    .entry(projection_ref.0.clone())
                    .or_insert(RecoverableInterfaceProjectionIdentityFact {
                        interface_type: serde_json::from_str(&interface.interface_abi_id)
                            .unwrap_or_else(|_| TypeRefIr::native(&interface.interface_abi_id)),
                        implemented_by: None,
                        interface_abi_id: Some(interface.interface_abi_id.clone()),
                    });
                push_unique(&mut plan.interface_projection_refs, projection_ref);
                if let Some((interface_module, decl)) =
                    self.interface_decl_for_instantiation(module_path, interface)
                {
                    let interface_module = interface_module.to_string();
                    let decl = decl.clone();
                    for operation in &decl.operations {
                        let method_abi_id =
                            recoverable_interface_method_abi_id(interface, operation);
                        let method_ref =
                            RecoverableInterfaceMethodIdentityRef(method_abi_id.clone());
                        let signature =
                            self.expected_type_plan(&interface_module, &operation.return_type);
                        self.metadata
                            .identity_tables
                            .interface_methods
                            .entry(method_ref.0.clone())
                            .or_insert(RecoverableInterfaceMethodIdentityFact {
                                interface_projection_ref: RecoverableInterfaceProjectionIdentityRef(
                                    format!("interface:{}", interface.interface_abi_id),
                                ),
                                method_name: operation.name.clone(),
                                method_abi_id: Some(method_abi_id),
                                signature: Some(signature),
                            });
                        push_unique(&mut plan.interface_method_refs, method_ref);
                    }
                }
                for arg in &interface.canonical_type_args {
                    self.collect_type_plan_identities(module_path, arg, owner, plan);
                }
            }
            TypeRefIr::Native { args, .. } => {
                for arg in args {
                    self.collect_type_plan_identities(module_path, arg, owner, plan);
                }
            }
            TypeRefIr::Nullable { inner } => {
                self.collect_type_plan_identities(module_path, inner, owner, plan)
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.collect_type_plan_identities(module_path, &param.ty, owner, plan);
                }
                self.collect_type_plan_identities(module_path, return_type, owner, plan);
            }
            TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => {}
        }
    }

    fn validate_expected_type_plan_closure(
        &self,
        metadata: &RecoverableArtifactMetadata,
        plan: &RecoverableExpectedTypePlan,
        boundary_kind: BoundaryKind,
        context: &str,
    ) -> Result<(), ProjectionError> {
        match &plan.root {
            RecoverableExpectedTypeRoot::TypeRef { ty } => {
                self.validate_expected_type_root_type_ref(ty, boundary_kind, context)
            }
            RecoverableExpectedTypeRoot::TypeIdentityRef { type_identity_ref } => {
                let Some(identity) = metadata.identity_tables.types.get(&type_identity_ref.0)
                else {
                    return Err(ProjectionError::ContractValidation {
                        message: format!(
                            "{context}: type identity `{}` cannot be resolved for durable state closure validation",
                            type_identity_ref.0
                        ),
                    });
                };
                // TODO(recoverable-p1): identity-only durable roots should carry
                // source module/type provenance. Until that exists, resolving the
                // stored TypeRef fact is mandatory and unresolved identities fail
                // closed so TypeIdentityRef cannot bypass closure policy.
                self.validate_expected_type_root_type_ref(&identity.ty, boundary_kind, context)
            }
        }
    }

    fn validate_expected_type_root_type_ref(
        &self,
        ty: &TypeRefIr,
        boundary_kind: BoundaryKind,
        context: &str,
    ) -> Result<(), ProjectionError> {
        if matches!(ty, TypeRefIr::LocalType { .. }) {
            let mut matched = false;
            for unit in self.file_ir_units.iter().chain(
                self.package_sources
                    .iter()
                    .flat_map(|package| package.file_ir_units.iter()),
            ) {
                if let TypeRefIr::LocalType { type_index } = ty {
                    if unit.type_table.get(*type_index as usize).is_none() {
                        continue;
                    }
                }
                matched = true;
                self.validate_type_ref_closure(&unit.module_path, ty, boundary_kind, context)?;
            }
            if matched {
                return Ok(());
            }
        }
        self.validate_type_ref_closure("", ty, boundary_kind, context)
    }

    fn validate_type_ref_closure(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        boundary_kind: BoundaryKind,
        context: &str,
    ) -> Result<(), ProjectionError> {
        self.validate_type_ref_closure_inner(
            module_path,
            ty,
            boundary_kind,
            context,
            &mut Vec::new(),
            &mut BTreeSet::new(),
        )
    }

    fn validate_type_ref_closure_inner(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        boundary_kind: BoundaryKind,
        context: &str,
        trace: &mut Vec<String>,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), ProjectionError> {
        validate_static_type_ref_boundary_policy(ty, boundary_kind)
            .map_err(|message| recoverable_closure_error(context, trace, message))?;

        match ty {
            TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::PackageSymbol { .. } => {
                let Some((resolved_module, decl)) =
                    self.type_decl_for_type_ref_with_module(module_path, ty)
                else {
                    return Err(recoverable_closure_error(
                        context,
                        trace,
                        format!(
                            "{} cannot be resolved for recoverable closure validation",
                            recoverable_nominal_type_label(ty)
                        ),
                    ));
                };
                let resolved = (resolved_module.to_string(), decl.clone());
                let identity_ref = self.nominal_type_identity_ref(module_path, ty, Some(&resolved));
                if let Some((plan_key, plan)) = self.custom_restore_plan_for_identity(&identity_ref)
                {
                    self.validate_expected_type_plan_closure(
                        &self.metadata,
                        &plan.durable_state_type_plan,
                        boundary_kind,
                        &format!("{context}: custom restore plan {plan_key} durable state type"),
                    )?;
                    return Ok(());
                }
                let key = format!("{resolved_module}.{}", decl.name);
                if !seen.insert(key.clone()) {
                    return Ok(());
                }
                trace.push(format!("type {key}"));
                self.validate_type_descriptor_closure_inner(
                    resolved_module,
                    &decl.descriptor,
                    boundary_kind,
                    context,
                    trace,
                    seen,
                )?;
                trace.pop();
                seen.remove(&key);
            }
            TypeRefIr::Record { fields } => {
                for (name, field_ty) in fields {
                    trace.push(format!("field {name}"));
                    self.validate_type_ref_closure_inner(
                        module_path,
                        field_ty,
                        boundary_kind,
                        context,
                        trace,
                        seen,
                    )?;
                    trace.pop();
                }
            }
            TypeRefIr::Union { items } => {
                for (index, item) in items.iter().enumerate() {
                    trace.push(format!("variant {index}"));
                    self.validate_type_ref_closure_inner(
                        module_path,
                        item,
                        boundary_kind,
                        context,
                        trace,
                        seen,
                    )?;
                    trace.pop();
                }
            }
            TypeRefIr::Nullable { inner } => {
                trace.push("nullable inner".to_string());
                self.validate_type_ref_closure_inner(
                    module_path,
                    inner,
                    boundary_kind,
                    context,
                    trace,
                    seen,
                )?;
                trace.pop();
            }
            TypeRefIr::Native { name, args } => {
                self.validate_native_type_ref_closure_inner(
                    module_path,
                    name,
                    args,
                    boundary_kind,
                    context,
                    trace,
                    seen,
                )?;
            }
            TypeRefIr::AnyInterface { interface } => {
                for (index, arg) in interface.canonical_type_args.iter().enumerate() {
                    trace.push(format!("interface type argument {index}"));
                    self.validate_type_ref_closure_inner(
                        module_path,
                        arg,
                        boundary_kind,
                        context,
                        trace,
                        seen,
                    )?;
                    trace.pop();
                }
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    trace.push(format!("param {}", param.name));
                    self.validate_type_ref_closure_inner(
                        module_path,
                        &param.ty,
                        boundary_kind,
                        context,
                        trace,
                        seen,
                    )?;
                    trace.pop();
                }
                trace.push("return type".to_string());
                self.validate_type_ref_closure_inner(
                    module_path,
                    return_type,
                    boundary_kind,
                    context,
                    trace,
                    seen,
                )?;
                trace.pop();
            }
            TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => {}
        }
        Ok(())
    }

    fn validate_type_descriptor_closure_inner(
        &self,
        module_path: &str,
        descriptor: &TypeDescriptorIr,
        boundary_kind: BoundaryKind,
        context: &str,
        trace: &mut Vec<String>,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), ProjectionError> {
        match descriptor {
            TypeDescriptorIr::Alias { target } => {
                trace.push("alias target".to_string());
                self.validate_type_ref_closure_inner(
                    module_path,
                    target,
                    boundary_kind,
                    context,
                    trace,
                    seen,
                )?;
                trace.pop();
            }
            TypeDescriptorIr::Record { fields } => {
                for (name, field_ty) in fields {
                    trace.push(format!("field {name}"));
                    self.validate_type_ref_closure_inner(
                        module_path,
                        field_ty,
                        boundary_kind,
                        context,
                        trace,
                        seen,
                    )?;
                    trace.pop();
                }
            }
            TypeDescriptorIr::Union { variants } => {
                for (index, variant) in variants.iter().enumerate() {
                    trace.push(format!("variant {index}"));
                    self.validate_type_ref_closure_inner(
                        module_path,
                        variant,
                        boundary_kind,
                        context,
                        trace,
                        seen,
                    )?;
                    trace.pop();
                }
            }
            TypeDescriptorIr::Native { symbol } => {
                if let Some((plan_key, plan)) = self.native_adapter_plan_for_symbol(symbol) {
                    self.validate_expected_type_plan_closure(
                        &self.metadata,
                        &plan.durable_state_type_plan,
                        boundary_kind,
                        &format!("{context}: native adapter plan {plan_key} durable state type"),
                    )?;
                } else {
                    return Err(recoverable_closure_error(
                        context,
                        trace,
                        format!(
                            "native descriptor `{symbol}` requires RecoverableNativeAdapterPlan"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_native_type_ref_closure_inner(
        &self,
        module_path: &str,
        name: &str,
        args: &[TypeRefIr],
        boundary_kind: BoundaryKind,
        context: &str,
        trace: &mut Vec<String>,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), ProjectionError> {
        match name {
            "string" | "integer" | "number" | "bool" | "boolean" | "null" | "void" | "Date"
            | "Duration" | "Bytes" | "Json" | "JsonObject" => {
                if args.is_empty() {
                    return Ok(());
                }
                return Err(recoverable_closure_error(
                    context,
                    trace,
                    format!("plain native type `{name}` cannot have type arguments"),
                ));
            }
            "Array" => {
                let [item] = args else {
                    return Err(recoverable_closure_error(
                        context,
                        trace,
                        "Array<T> must have exactly one type argument".to_string(),
                    ));
                };
                trace.push("array item".to_string());
                self.validate_type_ref_closure_inner(
                    module_path,
                    item,
                    boundary_kind,
                    context,
                    trace,
                    seen,
                )?;
                trace.pop();
            }
            "Map" => {
                let [key, value] = args else {
                    return Err(recoverable_closure_error(
                        context,
                        trace,
                        "Map<K,V> must have exactly two type arguments".to_string(),
                    ));
                };
                trace.push("map key".to_string());
                self.validate_recoverable_map_key_type(module_path, key, context, trace)?;
                trace.pop();
                trace.push("map value".to_string());
                self.validate_type_ref_closure_inner(
                    module_path,
                    value,
                    boundary_kind,
                    context,
                    trace,
                    seen,
                )?;
                trace.pop();
            }
            _ => {
                let ty = TypeRefIr::Native {
                    name: name.to_string(),
                    args: args.to_vec(),
                };
                if let Some((plan_key, plan)) = self.native_adapter_plan_for_type_ref(&ty) {
                    self.validate_expected_type_plan_closure(
                        &self.metadata,
                        &plan.durable_state_type_plan,
                        boundary_kind,
                        &format!("{context}: native adapter plan {plan_key} durable state type"),
                    )?;
                } else {
                    return Err(recoverable_closure_error(
                        context,
                        trace,
                        format!(
                            "native type `{}` requires RecoverableNativeAdapterPlan",
                            display_native_type_ref(name, args)
                        ),
                    ));
                }
                for (index, arg) in args.iter().enumerate() {
                    trace.push(format!("type argument {index}"));
                    self.validate_type_ref_closure_inner(
                        module_path,
                        arg,
                        boundary_kind,
                        context,
                        trace,
                        seen,
                    )?;
                    trace.pop();
                }
            }
        }
        Ok(())
    }

    fn validate_recoverable_map_key_type(
        &self,
        module_path: &str,
        key: &TypeRefIr,
        context: &str,
        trace: &[String],
    ) -> Result<(), ProjectionError> {
        self.validate_recoverable_map_key_type_inner(
            module_path,
            key,
            context,
            trace,
            &mut BTreeSet::new(),
        )
    }

    fn validate_recoverable_map_key_type_inner(
        &self,
        module_path: &str,
        key: &TypeRefIr,
        context: &str,
        trace: &[String],
        seen: &mut BTreeSet<String>,
    ) -> Result<(), ProjectionError> {
        match key {
            TypeRefIr::Native { name, args } if name == "string" && args.is_empty() => Ok(()),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::PackageSymbol { .. } => {
                let Some((resolved_module, decl)) =
                    self.type_decl_for_type_ref_with_module(module_path, key)
                else {
                    return Err(recoverable_closure_error(
                        context,
                        trace,
                        format!(
                            "{} cannot be resolved for recoverable map key validation",
                            recoverable_nominal_type_label(key)
                        ),
                    ));
                };
                let seen_key = format!("{resolved_module}.{}", decl.name);
                if !seen.insert(seen_key.clone()) {
                    return Err(recoverable_closure_error(
                        context,
                        trace,
                        format!(
                            "recursive map key type `{seen_key}` cannot be proven string-representable"
                        ),
                    ));
                }
                let result = match &decl.descriptor {
                    TypeDescriptorIr::Alias { target } => self
                        .validate_recoverable_map_key_type_inner(
                            resolved_module,
                            target,
                            context,
                            trace,
                            seen,
                        ),
                    TypeDescriptorIr::Record { .. }
                    | TypeDescriptorIr::Union { .. }
                    | TypeDescriptorIr::Native { .. } => Err(recoverable_closure_error(
                        context,
                        trace,
                        format!(
                            "map key {} must be string or nominal alias over string",
                            recoverable_nominal_type_label(key)
                        ),
                    )),
                };
                seen.remove(&seen_key);
                result
            }
            _ => Err(recoverable_closure_error(
                context,
                trace,
                format!(
                    "map key type `{}` must be string or nominal alias over string",
                    display_policy_type_ref_for_recoverable(key)
                ),
            )),
        }
    }

    fn contains_any_interface_closure(&self, module_path: &str, ty: &TypeRefIr) -> bool {
        self.contains_any_interface_closure_inner(module_path, ty, &mut BTreeSet::new())
    }

    fn contains_any_interface_closure_inner(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<String>,
    ) -> bool {
        match ty {
            TypeRefIr::AnyInterface { .. } => true,
            TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::PackageSymbol { .. } => {
                let Some((resolved_module, decl)) =
                    self.type_decl_for_type_ref_with_module(module_path, ty)
                else {
                    return true;
                };
                let resolved = (resolved_module.to_string(), decl.clone());
                let identity_ref = self.nominal_type_identity_ref(module_path, ty, Some(&resolved));
                if let Some((_, plan)) = self.custom_restore_plan_for_identity(&identity_ref) {
                    return self
                        .expected_type_plan_contains_any_interface(&plan.durable_state_type_plan);
                }
                let key = format!("{resolved_module}.{}", decl.name);
                if !seen.insert(key.clone()) {
                    return false;
                }
                let result = self.type_descriptor_contains_any_interface(
                    resolved_module,
                    &decl.descriptor,
                    seen,
                );
                seen.remove(&key);
                result
            }
            TypeRefIr::Native { args, .. } => args
                .iter()
                .any(|arg| self.contains_any_interface_closure_inner(module_path, arg, seen)),
            TypeRefIr::Record { fields } => fields.values().any(|field_ty| {
                self.contains_any_interface_closure_inner(module_path, field_ty, seen)
            }),
            TypeRefIr::Union { items } => items
                .iter()
                .any(|item| self.contains_any_interface_closure_inner(module_path, item, seen)),
            TypeRefIr::Nullable { inner } => {
                self.contains_any_interface_closure_inner(module_path, inner, seen)
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                params.iter().any(|param| {
                    self.contains_any_interface_closure_inner(module_path, &param.ty, seen)
                }) || self.contains_any_interface_closure_inner(module_path, return_type, seen)
            }
            TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => false,
        }
    }

    fn expected_type_plan_contains_any_interface(
        &self,
        plan: &RecoverableExpectedTypePlan,
    ) -> bool {
        plan.runtime_carrier_check_required
            || match &plan.root {
                RecoverableExpectedTypeRoot::TypeRef { ty } => {
                    self.contains_any_interface_closure("", ty)
                }
                RecoverableExpectedTypeRoot::TypeIdentityRef { .. } => false,
            }
    }

    fn type_descriptor_contains_any_interface(
        &self,
        module_path: &str,
        descriptor: &TypeDescriptorIr,
        seen: &mut BTreeSet<String>,
    ) -> bool {
        match descriptor {
            TypeDescriptorIr::Alias { target } => {
                self.contains_any_interface_closure_inner(module_path, target, seen)
            }
            TypeDescriptorIr::Record { fields } => fields.values().any(|field_ty| {
                self.contains_any_interface_closure_inner(module_path, field_ty, seen)
            }),
            TypeDescriptorIr::Union { variants } => variants.iter().any(|variant| {
                self.contains_any_interface_closure_inner(module_path, variant, seen)
            }),
            TypeDescriptorIr::Native { .. } => false,
        }
    }

    fn nominal_type_identity_ref(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        resolved: Option<&(String, TypeDeclIr)>,
    ) -> RecoverableTypeIdentityRef {
        match ty {
            TypeRefIr::LocalType { type_index } => {
                let resolved_module = resolved
                    .map(|(resolved_module, _)| resolved_module.as_str())
                    .unwrap_or(module_path);
                let symbol = resolved
                    .and_then(|(resolved_module, decl)| {
                        self.type_source_symbol_for_decl(resolved_module, decl)
                    })
                    .or_else(|| resolved.map(|(_, decl)| decl.name.as_str()))
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("unresolved-local-type-{type_index}"));
                RecoverableTypeIdentityRef(format!(
                    "type:source:{}:{symbol}",
                    self.module_identity_scope(resolved_module)
                ))
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                RecoverableTypeIdentityRef(format!(
                    "type:source:{}:{}",
                    self.module_identity_scope(&symbol.module_path),
                    symbol.symbol
                ))
            }
            TypeRefIr::PackageSymbol { symbol } => RecoverableTypeIdentityRef(format!(
                "type:package:{}:{}",
                self.canonical_package_id_for_ref(&symbol.package),
                symbol.symbol_path
            )),
            _ => structural_type_identity_ref(ty),
        }
    }

    fn module_identity_scope(&self, module_path: &str) -> String {
        if let Some(package) = self.package_source_containing_module(module_path) {
            format!("package:{}:module:{module_path}", package.package_id)
        } else {
            format!("module:{module_path}")
        }
    }

    fn canonical_package_id_for_ref(&self, package_ref: &PackageRefIr) -> String {
        if let Some(package) = self.package_source_for_ref(package_ref) {
            return package.package_id.clone();
        }
        match package_ref {
            PackageRefIr::PackageId { package_id } => package_id.clone(),
            PackageRefIr::Dependency { dependency_ref } => format!("dependency:{dependency_ref}"),
        }
    }

    fn type_source_symbol_for_decl(&self, module_path: &str, decl: &TypeDeclIr) -> Option<&str> {
        self.file_ir_unit_by_module_path(module_path)
            .and_then(|unit| {
                unit.declarations
                    .types
                    .values()
                    .find(|declaration| {
                        unit.type_table
                            .get(declaration.type_index as usize)
                            .map_or(false, |candidate| candidate == decl)
                    })
                    .map(|declaration| declaration.symbol.as_str())
            })
    }

    fn custom_restore_plan_for_identity(
        &self,
        identity_ref: &RecoverableTypeIdentityRef,
    ) -> Option<(&str, &RecoverableCustomRestorePlan)> {
        self.metadata
            .custom_restore_plans
            .iter()
            .find(|(_, plan)| plan.concrete_type_identity == identity_ref.0)
            .map(|(key, plan)| (key.as_str(), plan))
    }

    fn native_adapter_plan_for_type_ref(
        &self,
        ty: &TypeRefIr,
    ) -> Option<(&str, &RecoverableNativeAdapterPlan)> {
        let candidates = recoverable_native_type_identity_candidates(ty);
        self.metadata
            .native_adapter_plans
            .iter()
            .find(|(_, plan)| {
                candidates
                    .iter()
                    .any(|candidate| candidate == &plan.native_type_identity)
            })
            .map(|(key, plan)| (key.as_str(), plan))
    }

    fn native_adapter_plan_for_symbol(
        &self,
        symbol: &str,
    ) -> Option<(&str, &RecoverableNativeAdapterPlan)> {
        self.native_adapter_plan_for_type_ref(&TypeRefIr::native(symbol))
    }

    fn first_custom_restore_plan_ref(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Option<RecoverableCustomRestorePlanRef> {
        self.first_custom_restore_plan_ref_inner(module_path, ty, &mut BTreeSet::new())
    }

    fn first_custom_restore_plan_ref_inner(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<String>,
    ) -> Option<RecoverableCustomRestorePlanRef> {
        match ty {
            TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::PackageSymbol { .. } => {
                let (resolved_module, decl) =
                    self.type_decl_for_type_ref_with_module(module_path, ty)?;
                let resolved = (resolved_module.to_string(), decl.clone());
                let identity_ref = self.nominal_type_identity_ref(module_path, ty, Some(&resolved));
                if let Some((plan_key, _)) = self.custom_restore_plan_for_identity(&identity_ref) {
                    return Some(RecoverableCustomRestorePlanRef(plan_key.to_string()));
                }
                let seen_key = format!("{resolved_module}.{}", decl.name);
                if !seen.insert(seen_key.clone()) {
                    return None;
                }
                let result = self.first_custom_restore_plan_ref_for_descriptor(
                    resolved_module,
                    &decl.descriptor,
                    seen,
                );
                seen.remove(&seen_key);
                result
            }
            TypeRefIr::Record { fields } => fields.values().find_map(|field_ty| {
                self.first_custom_restore_plan_ref_inner(module_path, field_ty, seen)
            }),
            TypeRefIr::Union { items } => items
                .iter()
                .find_map(|item| self.first_custom_restore_plan_ref_inner(module_path, item, seen)),
            TypeRefIr::Nullable { inner } => {
                self.first_custom_restore_plan_ref_inner(module_path, inner, seen)
            }
            TypeRefIr::Native { args, .. } => args
                .iter()
                .find_map(|arg| self.first_custom_restore_plan_ref_inner(module_path, arg, seen)),
            TypeRefIr::Function {
                params,
                return_type,
            } => params
                .iter()
                .find_map(|param| {
                    self.first_custom_restore_plan_ref_inner(module_path, &param.ty, seen)
                })
                .or_else(|| {
                    self.first_custom_restore_plan_ref_inner(module_path, return_type, seen)
                }),
            TypeRefIr::AnyInterface { interface } => interface
                .canonical_type_args
                .iter()
                .find_map(|arg| self.first_custom_restore_plan_ref_inner(module_path, arg, seen)),
            TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => None,
        }
    }

    fn first_custom_restore_plan_ref_for_descriptor(
        &self,
        module_path: &str,
        descriptor: &TypeDescriptorIr,
        seen: &mut BTreeSet<String>,
    ) -> Option<RecoverableCustomRestorePlanRef> {
        match descriptor {
            TypeDescriptorIr::Alias { target } => {
                self.first_custom_restore_plan_ref_inner(module_path, target, seen)
            }
            TypeDescriptorIr::Record { fields } => fields.values().find_map(|field_ty| {
                self.first_custom_restore_plan_ref_inner(module_path, field_ty, seen)
            }),
            TypeDescriptorIr::Union { variants } => variants.iter().find_map(|variant| {
                self.first_custom_restore_plan_ref_inner(module_path, variant, seen)
            }),
            TypeDescriptorIr::Native { .. } => None,
        }
    }

    fn first_native_adapter_plan_ref(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Option<RecoverableNativeAdapterPlanRef> {
        self.first_native_adapter_plan_ref_inner(module_path, ty, &mut BTreeSet::new())
    }

    fn first_native_adapter_plan_ref_inner(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<String>,
    ) -> Option<RecoverableNativeAdapterPlanRef> {
        match ty {
            TypeRefIr::Native { args, .. } => self
                .native_adapter_plan_for_type_ref(ty)
                .map(|(plan_key, _)| RecoverableNativeAdapterPlanRef(plan_key.to_string()))
                .or_else(|| {
                    args.iter().find_map(|arg| {
                        self.first_native_adapter_plan_ref_inner(module_path, arg, seen)
                    })
                }),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::PackageSymbol { .. } => {
                let (resolved_module, decl) =
                    self.type_decl_for_type_ref_with_module(module_path, ty)?;
                let resolved = (resolved_module.to_string(), decl.clone());
                let identity_ref = self.nominal_type_identity_ref(module_path, ty, Some(&resolved));
                if self
                    .custom_restore_plan_for_identity(&identity_ref)
                    .is_some()
                {
                    return None;
                }
                let seen_key = format!("{resolved_module}.{}", decl.name);
                if !seen.insert(seen_key.clone()) {
                    return None;
                }
                let result = self.first_native_adapter_plan_ref_for_descriptor(
                    resolved_module,
                    &decl.descriptor,
                    seen,
                );
                seen.remove(&seen_key);
                result
            }
            TypeRefIr::Record { fields } => fields.values().find_map(|field_ty| {
                self.first_native_adapter_plan_ref_inner(module_path, field_ty, seen)
            }),
            TypeRefIr::Union { items } => items
                .iter()
                .find_map(|item| self.first_native_adapter_plan_ref_inner(module_path, item, seen)),
            TypeRefIr::Nullable { inner } => {
                self.first_native_adapter_plan_ref_inner(module_path, inner, seen)
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => params
                .iter()
                .find_map(|param| {
                    self.first_native_adapter_plan_ref_inner(module_path, &param.ty, seen)
                })
                .or_else(|| {
                    self.first_native_adapter_plan_ref_inner(module_path, return_type, seen)
                }),
            TypeRefIr::AnyInterface { interface } => interface
                .canonical_type_args
                .iter()
                .find_map(|arg| self.first_native_adapter_plan_ref_inner(module_path, arg, seen)),
            TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => None,
        }
    }

    fn first_native_adapter_plan_ref_for_descriptor(
        &self,
        module_path: &str,
        descriptor: &TypeDescriptorIr,
        seen: &mut BTreeSet<String>,
    ) -> Option<RecoverableNativeAdapterPlanRef> {
        match descriptor {
            TypeDescriptorIr::Alias { target } => {
                self.first_native_adapter_plan_ref_inner(module_path, target, seen)
            }
            TypeDescriptorIr::Record { fields } => fields.values().find_map(|field_ty| {
                self.first_native_adapter_plan_ref_inner(module_path, field_ty, seen)
            }),
            TypeDescriptorIr::Union { variants } => variants.iter().find_map(|variant| {
                self.first_native_adapter_plan_ref_inner(module_path, variant, seen)
            }),
            TypeDescriptorIr::Native { symbol } => self
                .native_adapter_plan_for_symbol(symbol)
                .map(|(plan_key, _)| RecoverableNativeAdapterPlanRef(plan_key.to_string())),
        }
    }

    fn collect_descriptor_identities(
        &mut self,
        module_path: &str,
        owner: &RecoverableTypeIdentityRef,
        descriptor: &TypeDescriptorIr,
        plan: &mut RecoverableExpectedTypePlan,
    ) {
        match descriptor {
            TypeDescriptorIr::Record { fields } => {
                for (name, field_ty) in fields {
                    let field_ref =
                        RecoverableFieldIdentityRef(format!("field:{}:{}", owner.0, name));
                    self.metadata
                        .identity_tables
                        .fields
                        .entry(field_ref.0.clone())
                        .or_insert(RecoverableFieldIdentityFact {
                            owner_type_ref: owner.clone(),
                            field_name: name.clone(),
                            field_type: Some(field_ty.clone()),
                            field_abi_id: Some(format!("{}#{name}", type_ref_abi_key(field_ty))),
                        });
                    push_unique(&mut plan.field_refs, field_ref);
                    self.collect_type_plan_identities(module_path, field_ty, Some(owner), plan);
                }
            }
            TypeDescriptorIr::Union { variants } => {
                for (index, variant) in variants.iter().enumerate() {
                    let branch_ref =
                        RecoverableUnionBranchIdentityRef(format!("union:{}:{index}", owner.0));
                    self.metadata
                        .identity_tables
                        .union_branches
                        .entry(branch_ref.0.clone())
                        .or_insert(RecoverableUnionBranchIdentityFact {
                            union_type_ref: owner.clone(),
                            branch_index: index as u32,
                            branch_type: variant.clone(),
                            branch_abi_id: Some(type_ref_abi_key(variant)),
                        });
                    push_unique(&mut plan.union_branch_refs, branch_ref);
                    self.collect_type_plan_identities(module_path, variant, Some(owner), plan);
                }
            }
            TypeDescriptorIr::Alias { target } => {
                self.collect_type_plan_identities(module_path, target, Some(owner), plan);
            }
            TypeDescriptorIr::Native { .. } => {}
        }
    }

    fn type_decl_for_type_ref_with_module(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Option<(&str, &TypeDeclIr)> {
        match ty {
            TypeRefIr::LocalType { type_index } => self
                .file_ir_unit_by_module_path(module_path)
                .and_then(|unit| {
                    unit.type_table
                        .get(*type_index as usize)
                        .map(|decl| (unit.module_path.as_str(), decl))
                }),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => self
                .file_ir_unit_by_module_path(&symbol.module_path)
                .and_then(|unit| {
                    unit.declarations
                        .types
                        .get(&symbol.symbol)
                        .and_then(|decl| unit.type_table.get(decl.type_index as usize))
                        .map(|decl| (unit.module_path.as_str(), decl))
                }),
            TypeRefIr::PackageSymbol { symbol } => self
                .type_decl_for_package_symbol_source(symbol)
                .or_else(|| self.type_decl_for_package_db_symbol_from_file_ir(module_path, symbol)),
            TypeRefIr::Native { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::Function { .. } => None,
        }
    }

    fn type_decl_for_package_symbol_source(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Option<(&str, &TypeDeclIr)> {
        let package = self.package_source_for_ref(&symbol.package)?;
        let export = package
            .unit
            .implementation_links
            .types
            .get(&symbol.symbol_path)?;
        let unit = package
            .file_ir_units
            .iter()
            .find(|unit| unit.file_ir_identity == export.file.file_ir_identity)
            .or_else(|| {
                package
                    .file_ir_units
                    .iter()
                    .find(|unit| unit.module_path == export.file.module_path)
            })?;
        unit.type_table
            .get(export.type_index as usize)
            .map(|decl| (unit.module_path.as_str(), decl))
    }

    fn type_decl_for_package_db_symbol_from_file_ir(
        &self,
        module_path: &str,
        symbol: &PackageSymbolRef,
    ) -> Option<(&str, &TypeDeclIr)> {
        if !self.package_symbol_matches_package_db_module(module_path, symbol) {
            return None;
        }
        let symbol_path = symbol.symbol_path.as_str();
        if let Some((symbol_module, symbol_name)) = symbol_path.rsplit_once('.') {
            if let Some(resolved) = self.type_decl_for_symbol_in_module(symbol_module, symbol_name)
            {
                return Some(resolved);
            }
        }
        if let Some(resolved) = self.type_decl_for_symbol_in_module(module_path, symbol_path) {
            return Some(resolved);
        }
        let short_name = symbol_path.rsplit('.').next().unwrap_or(symbol_path);
        if short_name != symbol_path {
            if let Some(resolved) = self.type_decl_for_symbol_in_module(module_path, short_name) {
                return Some(resolved);
            }
        }
        None
    }

    fn package_symbol_matches_package_db_module(
        &self,
        module_path: &str,
        symbol: &PackageSymbolRef,
    ) -> bool {
        if self.ambiguous_package_db_modules.contains(module_path) {
            return false;
        }
        let Some(package_id) = self.package_db_modules.get(module_path) else {
            return false;
        };
        match &symbol.package {
            PackageRefIr::PackageId {
                package_id: symbol_package_id,
            } => symbol_package_id == package_id,
            PackageRefIr::Dependency { dependency_ref } => dependency_ref == package_id,
        }
    }

    fn type_decl_for_symbol_in_module(
        &self,
        module_path: &str,
        symbol_name: &str,
    ) -> Option<(&str, &TypeDeclIr)> {
        let unit = self.file_ir_unit_by_module_path(module_path)?;
        type_decl_for_symbol_in_unit(unit, symbol_name)
            .map(|decl| (unit.module_path.as_str(), decl))
    }

    fn file_ir_unit_by_module_path(&self, module_path: &str) -> Option<&FileIrUnit> {
        self.file_ir_units
            .iter()
            .find(|unit| unit.module_path == module_path)
            .or_else(|| {
                self.package_sources
                    .iter()
                    .flat_map(|package| package.file_ir_units.iter())
                    .find(|unit| unit.module_path == module_path)
            })
    }

    fn package_source_containing_module(
        &self,
        module_path: &str,
    ) -> Option<&RecoverablePackageTypeSource> {
        self.package_sources.iter().find(|package| {
            package
                .file_ir_units
                .iter()
                .any(|unit| unit.module_path == module_path)
        })
    }

    fn package_source_for_ref(
        &self,
        package_ref: &PackageRefIr,
    ) -> Option<&RecoverablePackageTypeSource> {
        let package_key = match package_ref {
            PackageRefIr::PackageId { package_id } => package_id.as_str(),
            PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
        };
        self.package_sources.iter().find(|source| {
            source.package_id == package_key
                || source.unit.package_id == package_key
                || source
                    .dependency_refs
                    .iter()
                    .any(|dependency_ref| dependency_ref == package_key)
        })
    }

    fn module_path_for_spawn_target(&self, target: &SpawnTargetIr) -> Option<&str> {
        self.file_ir_units
            .iter()
            .find(|unit| {
                unit.file_ir_identity == target.executable_target.file_ref.file_ir_identity
                    || unit.module_path == target.executable_target.file_ref.module_path
            })
            .map(|unit| unit.module_path.as_str())
    }

    fn interface_decl_for_instantiation(
        &self,
        module_path: &str,
        interface: &InterfaceInstantiationRef,
    ) -> Option<(&str, &InterfaceDeclIr)> {
        if let Ok(interface_ty) = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id) {
            match interface_ty {
                TypeRefIr::ServiceSymbol { symbol } => {
                    return self
                        .file_ir_units
                        .iter()
                        .find(|unit| unit.module_path == symbol.module_path)
                        .and_then(|unit| {
                            unit.declarations
                                .interfaces
                                .get(&symbol.symbol)
                                .map(|decl| (unit.module_path.as_str(), decl))
                        });
                }
                TypeRefIr::PackageSymbol { symbol } => {
                    return self.interface_decl_for_package_symbol(&symbol);
                }
                TypeRefIr::Native { name, .. } => {
                    return self.interface_decl_by_module_name(module_path, &name);
                }
                _ => {}
            }
        }
        self.interface_decl_by_module_name(module_path, &interface.interface_abi_id)
    }

    fn interface_decl_for_package_symbol(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Option<(&str, &InterfaceDeclIr)> {
        let package = self.package_source_for_ref(&symbol.package)?;
        let export = package
            .unit
            .implementation_links
            .types
            .get(&symbol.symbol_path)?;
        let unit = package
            .file_ir_units
            .iter()
            .find(|unit| unit.file_ir_identity == export.file.file_ir_identity)
            .or_else(|| {
                package
                    .file_ir_units
                    .iter()
                    .find(|unit| unit.module_path == export.file.module_path)
            })?;
        if !export.symbol.is_empty() {
            if let Some(decl) = unit.declarations.interfaces.get(&export.symbol) {
                return Some((unit.module_path.as_str(), decl));
            }
        }
        unit.type_table
            .get(export.type_index as usize)
            .and_then(|decl| unit.declarations.interfaces.get(&decl.name))
            .map(|decl| (unit.module_path.as_str(), decl))
    }

    fn interface_decl_by_module_name(
        &self,
        module_path: &str,
        name: &str,
    ) -> Option<(&str, &InterfaceDeclIr)> {
        self.file_ir_unit_by_module_path(module_path)
            .and_then(|unit| {
                unit.declarations
                    .interfaces
                    .get(name)
                    .map(|decl| (unit.module_path.as_str(), decl))
            })
    }
}

fn db_field_lane_ref(db: &DbMetadataIr, field_name: &str) -> RecoverableStorageLaneRef {
    RecoverableStorageLaneRef(format!("db:{}:field:{}", db.type_name, field_name))
}

fn custom_restore_plan_key(plan: &RecoverableCustomRestorePlan) -> String {
    plan.concrete_type_identity.clone()
}

fn native_adapter_plan_key(plan: &RecoverableNativeAdapterPlan) -> String {
    plan.native_type_identity.clone()
}

fn db_storage_lane_for_type(ty: &TypeRefIr) -> RecoverableStorageLane {
    if schema_projectable_recoverable_type(ty) {
        RecoverableStorageLane::SchemaProjectable
    } else {
        RecoverableStorageLane::RecoverableEnvelope
    }
}

fn schema_projectable_recoverable_type(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Native { name, args } => match name.as_str() {
            "string" | "integer" | "number" | "bool" | "boolean" | "null" | "Date" | "Bytes"
            | "Json" | "JsonObject" => args.is_empty(),
            "Array" => {
                matches!(args.as_slice(), [item] if schema_projectable_recoverable_type(item))
            }
            "Map" => {
                matches!(args.as_slice(), [key, value] if is_plain_string_key(key) && schema_projectable_recoverable_type(value))
            }
            _ => false,
        },
        TypeRefIr::Record { fields } => fields.values().all(schema_projectable_recoverable_type),
        TypeRefIr::Union { items } => items.iter().all(schema_projectable_recoverable_type),
        TypeRefIr::Nullable { inner } => schema_projectable_recoverable_type(inner),
        TypeRefIr::Literal { .. } => true,
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => false,
    }
}

fn is_plain_string_key(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, args } if args.is_empty() && name == "string")
}

fn recoverable_behavior_nodes(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. } => true,
        TypeRefIr::Native { name, args } => {
            !matches!(
                name.as_str(),
                "string"
                    | "integer"
                    | "number"
                    | "bool"
                    | "boolean"
                    | "null"
                    | "Date"
                    | "Bytes"
                    | "Json"
                    | "JsonObject"
                    | "Array"
                    | "Map"
            ) || args.iter().any(recoverable_behavior_nodes)
        }
        TypeRefIr::Record { fields } => fields.values().any(recoverable_behavior_nodes),
        TypeRefIr::Union { items } => items.iter().any(recoverable_behavior_nodes),
        TypeRefIr::Nullable { inner } => recoverable_behavior_nodes(inner),
        TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => false,
    }
}

fn recoverable_closure_boundary_kind(context: &RecoverableBoundaryContext) -> BoundaryKind {
    if context.trust_boundary != RecoverableTrustBoundary::OwnerInternal {
        // Non-owner recoverable boundaries fail closed on behavior nodes before
        // closure validation. Use recoverable payload policy here so legacy
        // ordinary runtime/materialization schema bans do not define P1
        // recoverable semantics.
        return BoundaryKind::RecoverablePayload;
    }
    match context.boundary_kind {
        RecoverableBoundaryKind::DbPayload => BoundaryKind::DbPayload,
        RecoverableBoundaryKind::SpawnPayload => BoundaryKind::SpawnPayload,
        RecoverableBoundaryKind::QueuePayload => BoundaryKind::QueueWorkItemPayload,
        RecoverableBoundaryKind::RuntimeWirePayload
        | RecoverableBoundaryKind::ServicePayload
        | RecoverableBoundaryKind::PublicApiPayload
        | RecoverableBoundaryKind::Materialization => BoundaryKind::RecoverablePayload,
    }
}

fn recoverable_boundary_kind_label(kind: RecoverableBoundaryKind) -> &'static str {
    match kind {
        RecoverableBoundaryKind::DbPayload => "DB payload",
        RecoverableBoundaryKind::SpawnPayload => "spawn payload",
        RecoverableBoundaryKind::QueuePayload => "queue payload",
        RecoverableBoundaryKind::RuntimeWirePayload => "runtime wire payload",
        RecoverableBoundaryKind::ServicePayload => "service payload",
        RecoverableBoundaryKind::PublicApiPayload => "public API payload",
        RecoverableBoundaryKind::Materialization => "materialization",
    }
}

fn recoverable_closure_error(context: &str, trace: &[String], message: String) -> ProjectionError {
    let suffix = if trace.is_empty() {
        String::new()
    } else {
        format!(" via {}", trace.join(" -> "))
    };
    ProjectionError::ContractValidation {
        message: format!("{context}: {message}{suffix}"),
    }
}

fn recoverable_nominal_type_label(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::LocalType { type_index } => format!("local type index {type_index}"),
        TypeRefIr::ServiceSymbol { symbol } => {
            format!("service symbol {}", symbol.symbol_path())
        }
        TypeRefIr::DbObjectSymbol { symbol } => {
            format!("db object symbol {}", symbol.symbol_path())
        }
        TypeRefIr::PackageSymbol { symbol } => {
            format!("package symbol {}", symbol.symbol_path)
        }
        _ => format!("type {}", type_ref_abi_key(ty)),
    }
}

fn type_decl_for_symbol_in_unit<'a>(
    unit: &'a FileIrUnit,
    symbol_name: &str,
) -> Option<&'a TypeDeclIr> {
    unit.declarations
        .types
        .get(symbol_name)
        .and_then(|declaration| unit.type_table.get(declaration.type_index as usize))
        .or_else(|| unit.type_table.iter().find(|decl| decl.name == symbol_name))
}

fn recoverable_native_type_identity_candidates(ty: &TypeRefIr) -> Vec<String> {
    let mut candidates = vec![type_ref_abi_key(ty)];
    if let TypeRefIr::Native { name, args } = ty {
        candidates.push(format!("native:{name}"));
        if args.is_empty() {
            candidates.push(format!("native:{}", type_ref_abi_key(ty)));
        }
    }
    candidates
}

fn display_native_type_ref(name: &str, args: &[TypeRefIr]) -> String {
    if args.is_empty() {
        return name.to_string();
    }
    format!(
        "{name}<{}>",
        args.iter()
            .map(display_policy_type_ref_for_recoverable)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn display_policy_type_ref_for_recoverable(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Native { name, args } => display_native_type_ref(name, args),
        TypeRefIr::LocalType { type_index } => format!("local type index {type_index}"),
        TypeRefIr::ServiceSymbol { symbol } => symbol.symbol_path(),
        TypeRefIr::DbObjectSymbol { symbol } => symbol.symbol_path(),
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::Record { fields } => format!("record({} fields)", fields.len()),
        TypeRefIr::Union { items } => format!("union({} variants)", items.len()),
        TypeRefIr::Nullable { inner } => {
            format!("{}?", display_policy_type_ref_for_recoverable(inner))
        }
        TypeRefIr::Literal { value } => format!("literal {value:?}"),
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::AnyInterface { interface } => {
            format!("any interface {}", interface.interface_abi_id)
        }
        TypeRefIr::Function { .. } => "function".to_string(),
    }
}

fn structural_type_identity_ref(ty: &TypeRefIr) -> RecoverableTypeIdentityRef {
    RecoverableTypeIdentityRef(format!("type:structural:{}", type_ref_abi_key(ty)))
}

fn recoverable_interface_method_abi_id(
    interface: &InterfaceInstantiationRef,
    operation: &InterfaceOperationIr,
) -> String {
    let payload = (
        "recoverableInterfaceMethodV1",
        &interface.interface_abi_id,
        &interface.canonical_type_args,
        &operation.name,
        &operation.type_params,
        &operation.params,
        &operation.return_type,
    );
    format!(
        "recoverableInterfaceMethod:{}",
        serde_json::to_string(&payload).expect("recoverable interface method payload serializes")
    )
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, value: T) {
    if !items.contains(&value) {
        items.push(value);
    }
}

pub fn validate_db_recoverable_envelope_field_path(
    context: &str,
    path: &FieldPathIr,
    envelope_fields: &BTreeSet<&str>,
) -> Result<(), String> {
    // P5 wires this helper into projection/where/order/index validation. P1
    // owns the lane model and exposes the shared rule without touching DB runtime.
    let Some(first) = path.segments.first() else {
        return Ok(());
    };
    if envelope_fields.contains(first.as_str()) {
        if path.segments.len() > 1 {
            return Err(format!(
                "{context} cannot project through recoverable envelope field `{}` via `{}`",
                first, path.text
            ));
        } else {
            return Err(format!(
                "{context} cannot use recoverable envelope field `{}` in projection/where/order/index",
                path.text
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use skiff_artifact_model::{
        DbObjectFieldIr, DbObjectKindIr, FieldPathIr, FileIrRef, FileIrUnit, FunctionTypeParamIr,
        InterfaceDeclIr, InterfaceInstantiationRef, InterfaceOperationIr, LiteralIr,
        OperationCallableKind, OperationTargetRef, PackageRefIr, PackageSymbolRef, PackageUnit,
        RecoverableAdapterSchemaCompatibility, RecoverableCustomRestorePlan,
        RecoverableNativeAdapterOwner, RecoverableNativeAdapterPlan, RecoverableRestoreCapability,
        SpawnTargetKindIr, TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeExport,
    };

    fn any_interface() -> TypeRefIr {
        TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: "iface:ToolProvider".to_string(),
                canonical_type_args: Vec::new(),
            },
        }
    }

    fn callback_type() -> TypeRefIr {
        TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::native("string"),
            }],
            return_type: Box::new(TypeRefIr::native("string")),
        }
    }

    fn stream_type() -> TypeRefIr {
        TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::native("string")],
        }
    }

    fn host_handle_type() -> TypeRefIr {
        TypeRefIr::native("std.HostHandle")
    }

    fn map_type(key: TypeRefIr, value: TypeRefIr) -> TypeRefIr {
        TypeRefIr::Native {
            name: "Map".to_string(),
            args: vec![key, value],
        }
    }

    fn local_type_unit(name: &str, descriptor: TypeDescriptorIr) -> FileIrUnit {
        local_type_unit_with_module("app", name, descriptor)
    }

    fn local_type_unit_with_module(
        module_path: &str,
        name: &str,
        descriptor: TypeDescriptorIr,
    ) -> FileIrUnit {
        let mut unit = FileIrUnit::empty(module_path, "hash");
        unit.type_table.push(TypeDeclIr {
            name: name.to_string(),
            descriptor,
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        unit.declarations.types.insert(
            name.to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: name.to_string(),
                source_span: None,
            },
        );
        unit
    }

    fn spawn_target_with_param(ty: TypeRefIr) -> SpawnTargetIr {
        SpawnTargetIr {
            target_identity: "function:run".to_string(),
            kind: SpawnTargetKindIr::Function,
            executable_target: OperationTargetRef {
                file_ref: FileIrRef::new("file:app", "app"),
                executable_index: 0,
                callable_abi_id: "callable:run".to_string(),
                callable_kind: OperationCallableKind::InternalFunction,
            },
            param_types: vec![ty],
            return_type: Some(TypeRefIr::native("void")),
            service_protocol_identity: "protocol:svc".to_string(),
        }
    }

    fn type_identity_expected_plan(
        type_identity_ref: RecoverableTypeIdentityRef,
    ) -> RecoverableExpectedTypePlan {
        RecoverableExpectedTypePlan {
            root: RecoverableExpectedTypeRoot::TypeIdentityRef { type_identity_ref },
            root_type_identity_ref: None,
            runtime_carrier_check_required: false,
            interface_projection_refs: Vec::new(),
            interface_method_refs: Vec::new(),
            field_refs: Vec::new(),
            union_branch_refs: Vec::new(),
        }
    }

    fn package_source_with_exported_invalid_type() -> (RecoverablePackageTypeSource, TypeRefIr) {
        let mut file = FileIrUnit::empty("pkg.data", "hash");
        file.file_ir_identity = "file:pkg.data".to_string();
        file.type_table.push(TypeDeclIr {
            name: "PkgState".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("stream".to_string(), stream_type())]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "PkgState".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PkgState".to_string(),
                source_span: None,
            },
        );

        let mut unit = PackageUnit::empty("pkg.example", "0.1.0", "build:pkg", "abi:pkg");
        unit.implementation_links.types.insert(
            "PkgState".to_string(),
            TypeExport {
                file: FileIrRef::new("file:pkg.data", "pkg.data"),
                type_index: 0,
                symbol: "PkgState".to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        let ty = TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "pkg.example".to_string(),
                },
                symbol_path: "PkgState".to_string(),
                abi_expectation: None,
            },
        };
        (
            RecoverablePackageTypeSource {
                package_id: "pkg.example".to_string(),
                dependency_refs: Vec::new(),
                unit,
                file_ir_units: vec![file],
            },
            ty,
        )
    }

    fn package_source_with_exported_string_alias() -> (RecoverablePackageTypeSource, TypeRefIr) {
        package_source_with_exported_descriptor(
            "PkgKey",
            TypeDescriptorIr::Alias {
                target: TypeRefIr::native("string"),
            },
        )
    }

    fn package_source_with_exported_native_descriptor() -> (RecoverablePackageTypeSource, TypeRefIr)
    {
        package_source_with_exported_descriptor(
            "PkgNative",
            TypeDescriptorIr::Native {
                symbol: "std.HostHandle".to_string(),
            },
        )
    }

    fn package_source_with_exported_descriptor(
        name: &str,
        descriptor: TypeDescriptorIr,
    ) -> (RecoverablePackageTypeSource, TypeRefIr) {
        let mut file = FileIrUnit::empty("pkg.data", "hash");
        file.file_ir_identity = "file:pkg.data".to_string();
        file.type_table.push(TypeDeclIr {
            name: name.to_string(),
            descriptor,
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            name.to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: name.to_string(),
                source_span: None,
            },
        );

        let mut unit = PackageUnit::empty("pkg.example", "0.1.0", "build:pkg", "abi:pkg");
        unit.implementation_links.types.insert(
            name.to_string(),
            TypeExport {
                file: FileIrRef::new("file:pkg.data", "pkg.data"),
                type_index: 0,
                symbol: name.to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        let ty = TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "pkg.example".to_string(),
                },
                symbol_path: name.to_string(),
                abi_expectation: None,
            },
        };
        (
            RecoverablePackageTypeSource {
                package_id: "pkg.example".to_string(),
                dependency_refs: Vec::new(),
                unit,
                file_ir_units: vec![file],
            },
            ty,
        )
    }

    fn package_source_with_exported_interface(
        return_type: TypeRefIr,
    ) -> (RecoverablePackageTypeSource, TypeRefIr) {
        let mut file = FileIrUnit::empty("pkg.iface", "hash");
        file.file_ir_identity = "file:pkg.iface".to_string();
        file.type_table.push(TypeDeclIr {
            name: "PkgToolProvider".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "PkgToolProvider".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PkgToolProvider".to_string(),
                source_span: None,
            },
        );
        file.declarations.interfaces.insert(
            "PkgToolProvider".to_string(),
            InterfaceDeclIr {
                name: "PkgToolProvider".to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: "call".to_string(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type,
                    is_native: false,
                    is_provider: true,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );

        let mut unit = PackageUnit::empty("pkg.example", "0.1.0", "build:pkg", "abi:pkg");
        unit.implementation_links.types.insert(
            "PkgToolProvider".to_string(),
            TypeExport {
                file: FileIrRef::new("file:pkg.iface", "pkg.iface"),
                type_index: 0,
                symbol: "PkgToolProvider".to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        let ty = TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "pkg.example".to_string(),
                },
                symbol_path: "PkgToolProvider".to_string(),
                abi_expectation: None,
            },
        };
        (
            RecoverablePackageTypeSource {
                package_id: "pkg.example".to_string(),
                dependency_refs: Vec::new(),
                unit,
                file_ir_units: vec![file],
            },
            ty,
        )
    }

    fn custom_restore_plan(concrete_type_identity: &str) -> RecoverableCustomRestorePlan {
        RecoverableCustomRestorePlan {
            concrete_type_identity: concrete_type_identity.to_string(),
            restore_schema_version: "1".to_string(),
            durable_state_type_plan: RecoverableExpectedTypePlan {
                root: RecoverableExpectedTypeRoot::TypeRef {
                    ty: TypeRefIr::native("Json"),
                },
                root_type_identity_ref: None,
                runtime_carrier_check_required: false,
                interface_projection_refs: Vec::new(),
                interface_method_refs: Vec::new(),
                field_refs: Vec::new(),
                union_branch_refs: Vec::new(),
            },
            encode_hook_id: format!("{concrete_type_identity}.encode"),
            decode_hook_id: format!("{concrete_type_identity}.decode"),
            restore_capability: RecoverableRestoreCapability::Exact,
        }
    }

    fn native_adapter_plan(native_name: &str) -> RecoverableNativeAdapterPlan {
        RecoverableNativeAdapterPlan {
            adapter_identity: format!("adapter:{native_name}"),
            adapter_schema_version: "1".to_string(),
            native_type_identity: format!("native:{native_name}"),
            durable_state_type_plan: RecoverableExpectedTypePlan {
                root: RecoverableExpectedTypeRoot::TypeRef {
                    ty: TypeRefIr::native("Json"),
                },
                root_type_identity_ref: None,
                runtime_carrier_check_required: false,
                interface_projection_refs: Vec::new(),
                interface_method_refs: Vec::new(),
                field_refs: Vec::new(),
                union_branch_refs: Vec::new(),
            },
            encode_hook_id: format!("adapter:{native_name}.encode"),
            decode_hook_id: format!("adapter:{native_name}.decode"),
            owner: RecoverableNativeAdapterOwner {
                service_identity: "svc".to_string(),
            },
            schema_compatibility: RecoverableAdapterSchemaCompatibility::Exact,
        }
    }

    #[test]
    fn owner_internal_any_interface_marks_runtime_carrier_check() {
        let (_, plan) = recoverable_boundary_plan_for_type(
            "spawn:test",
            "",
            &[],
            &any_interface(),
            RecoverableBoundaryKind::SpawnPayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("owner-internal any interface should be accepted");

        assert!(plan.runtime_carrier_check_required);
        assert!(plan.expected_type.runtime_carrier_check_required);
        assert_eq!(
            plan.context.trust_boundary,
            RecoverableTrustBoundary::OwnerInternal
        );
    }

    #[test]
    fn owner_internal_runtime_wire_any_interface_marks_runtime_carrier_check() {
        let (_, plan) = recoverable_boundary_plan_for_type(
            "runtime:test",
            "",
            &[],
            &any_interface(),
            RecoverableBoundaryKind::RuntimeWirePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("owner-internal runtime wire recoverable boundary should accept any interface");

        assert!(plan.runtime_carrier_check_required);
        assert!(plan.expected_type.runtime_carrier_check_required);
        assert_eq!(
            plan.context.boundary_kind,
            RecoverableBoundaryKind::RuntimeWirePayload
        );
        assert_eq!(
            plan.context.trust_boundary,
            RecoverableTrustBoundary::OwnerInternal
        );
    }

    #[test]
    fn owner_internal_materialization_any_interface_marks_runtime_carrier_check() {
        let (_, plan) = recoverable_boundary_plan_for_type(
            "materialization:test",
            "",
            &[],
            &any_interface(),
            RecoverableBoundaryKind::Materialization,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("owner-internal materialization recoverable boundary should accept any interface");

        assert!(plan.runtime_carrier_check_required);
        assert!(plan.expected_type.runtime_carrier_check_required);
        assert_eq!(
            plan.context.boundary_kind,
            RecoverableBoundaryKind::Materialization
        );
        assert_eq!(
            plan.context.trust_boundary,
            RecoverableTrustBoundary::OwnerInternal
        );
    }

    #[test]
    fn public_explicit_slot_rejects_behavior_nodes() {
        let error = recoverable_boundary_plan_for_type(
            "public:test",
            "",
            &[],
            &any_interface(),
            RecoverableBoundaryKind::PublicApiPayload,
            RecoverableTrustBoundary::ExternalUntrusted,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("public behavior slot should fail closed");

        assert!(error.to_string().contains("behavior nodes"));
    }

    #[test]
    fn cross_service_runtime_wire_rejects_behavior_nodes() {
        let error = recoverable_boundary_plan_for_type(
            "runtime:test",
            "",
            &[],
            &any_interface(),
            RecoverableBoundaryKind::RuntimeWirePayload,
            RecoverableTrustBoundary::CrossService,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("cross-service runtime wire behavior slot should fail closed");

        assert!(error.to_string().contains("runtime wire payload"));
        assert!(error.to_string().contains("behavior nodes"));
    }

    #[test]
    fn external_materialization_rejects_behavior_nodes() {
        let error = recoverable_boundary_plan_for_type(
            "materialization:test",
            "",
            &[],
            &any_interface(),
            RecoverableBoundaryKind::Materialization,
            RecoverableTrustBoundary::ExternalUntrusted,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("external materialization behavior slot should fail closed");

        assert!(error.to_string().contains("materialization"));
        assert!(error.to_string().contains("behavior nodes"));
    }

    #[test]
    fn explicit_slot_builder_with_files_collects_local_record_field_refs() {
        let mut unit = FileIrUnit::empty("app", "hash");
        unit.type_table.push(TypeDeclIr {
            name: "SlotPayload".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("label".to_string(), TypeRefIr::native("string"))]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });

        let (_, plan) = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("explicit slot builder should accept owner-internal local record");

        assert!(plan
            .expected_type
            .field_refs
            .iter()
            .any(|field| field.0.contains(":label")));
    }

    #[test]
    fn explicit_slot_marks_runtime_check_for_local_nominal_nested_any_interface() {
        let unit = local_type_unit(
            "SlotPayload",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("provider".to_string(), any_interface())]),
            },
        );

        let (_, plan) = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("owner-internal local nominal with nested any interface should be accepted");

        assert!(plan.runtime_carrier_check_required);
        assert!(plan.expected_type.runtime_carrier_check_required);
    }

    #[test]
    fn explicit_slot_marks_runtime_check_for_package_nominal_nested_any_interface() {
        let (package_source, package_ty) = package_source_with_exported_descriptor(
            "PkgPayload",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("provider".to_string(), any_interface())]),
            },
        );

        let (_, plan) = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[],
            &package_ty,
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs {
                package_sources: &[package_source],
                ..RecoverableInputs::default()
            },
        )
        .expect("owner-internal package nominal with nested any interface should be accepted");

        assert!(plan.runtime_carrier_check_required);
        assert!(plan.expected_type.runtime_carrier_check_required);
    }

    #[test]
    fn local_type_identity_uses_module_and_symbol_not_type_index_only() {
        let unit_a = local_type_unit_with_module(
            "app.alpha",
            "SharedPayload",
            TypeDescriptorIr::Alias {
                target: TypeRefIr::native("string"),
            },
        );
        let unit_b = local_type_unit_with_module(
            "app.beta",
            "SharedPayload",
            TypeDescriptorIr::Alias {
                target: TypeRefIr::native("string"),
            },
        );

        let (_, plan_a) = recoverable_boundary_plan_for_type(
            "slot:a",
            "app.alpha",
            &[unit_a],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("alpha local type should build a recoverable plan");
        let (_, plan_b) = recoverable_boundary_plan_for_type(
            "slot:b",
            "app.beta",
            &[unit_b],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("beta local type should build a recoverable plan");

        let id_a = plan_a
            .expected_type
            .root_type_identity_ref
            .expect("alpha root identity")
            .0;
        let id_b = plan_b
            .expected_type
            .root_type_identity_ref
            .expect("beta root identity")
            .0;

        assert_ne!(id_a, id_b);
        assert!(id_a.contains("module:app.alpha:SharedPayload"));
        assert!(id_b.contains("module:app.beta:SharedPayload"));
        assert!(!id_a.contains("typeIndex"));
        assert!(!id_b.contains("typeIndex"));
    }

    #[test]
    fn local_type_identity_stays_symbol_derived_across_type_table_reorder() {
        let unit_before = local_type_unit_with_module(
            "app",
            "TargetPayload",
            TypeDescriptorIr::Alias {
                target: TypeRefIr::native("string"),
            },
        );

        let mut unit_after = FileIrUnit::empty("app", "hash");
        unit_after.type_table.push(TypeDeclIr {
            name: "OtherPayload".to_string(),
            descriptor: TypeDescriptorIr::Alias {
                target: TypeRefIr::native("number"),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        unit_after.type_table.push(TypeDeclIr {
            name: "TargetPayload".to_string(),
            descriptor: TypeDescriptorIr::Alias {
                target: TypeRefIr::native("string"),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        unit_after.declarations.types.insert(
            "OtherPayload".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "OtherPayload".to_string(),
                source_span: None,
            },
        );
        unit_after.declarations.types.insert(
            "TargetPayload".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "TargetPayload".to_string(),
                source_span: None,
            },
        );

        let (_, plan_before) = recoverable_boundary_plan_for_type(
            "slot:before",
            "app",
            &[unit_before],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("pre-reorder local type should build a recoverable plan");
        let (_, plan_after) = recoverable_boundary_plan_for_type(
            "slot:after",
            "app",
            &[unit_after],
            &TypeRefIr::LocalType { type_index: 1 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("post-reorder local type should build a recoverable plan");

        assert_eq!(
            plan_before.expected_type.root_type_identity_ref,
            plan_after.expected_type.root_type_identity_ref
        );
    }

    #[test]
    fn explicit_slot_rejects_local_record_nested_function() {
        let unit = local_type_unit(
            "SlotPayload",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("callback".to_string(), callback_type())]),
            },
        );

        let error = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("local record closure should reject nested function");

        assert!(error.to_string().contains("callback function type"));
        assert!(error.to_string().contains("field callback"));
    }

    #[test]
    fn explicit_slot_rejects_local_alias_nested_stream() {
        let unit = local_type_unit(
            "SlotPayload",
            TypeDescriptorIr::Alias {
                target: stream_type(),
            },
        );

        let error = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("local alias closure should reject nested stream");

        assert!(error.to_string().contains("Stream<T>"));
        assert!(error.to_string().contains("alias target"));
    }

    #[test]
    fn explicit_slot_rejects_local_union_nested_function() {
        let unit = local_type_unit(
            "SlotPayload",
            TypeDescriptorIr::Union {
                variants: vec![TypeRefIr::native("string"), callback_type()],
            },
        );

        let error = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("local union closure should reject nested function");

        assert!(error.to_string().contains("callback function type"));
        assert!(error.to_string().contains("variant 1"));
    }

    #[test]
    fn explicit_slot_rejects_native_without_adapter_plan() {
        let error = recoverable_boundary_plan_for_type(
            "slot:test",
            "",
            &[],
            &host_handle_type(),
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("non-plain native type should require a native adapter plan");

        assert!(error.to_string().contains("std.HostHandle"));
        assert!(error
            .to_string()
            .contains("requires RecoverableNativeAdapterPlan"));
    }

    #[test]
    fn declared_custom_restore_nominal_skips_raw_graph_and_sets_plan_ref() {
        let unit = local_type_unit(
            "Widget",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("callback".to_string(), callback_type())]),
            },
        );
        let custom_identity = "type:source:module:app:Widget";
        let custom_plan = custom_restore_plan(custom_identity);

        let (_, plan) = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit.clone()],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs {
                custom_restore_plans: std::slice::from_ref(&custom_plan),
                ..RecoverableInputs::default()
            },
        )
        .expect(
            "declared custom restore nominal should validate durable state instead of raw graph",
        );

        assert_eq!(
            plan.custom_restore_plan_ref,
            Some(RecoverableCustomRestorePlanRef(custom_identity.to_string()))
        );
        assert!(plan.native_adapter_plan_ref.is_none());
        assert!(plan.expected_type.field_refs.is_empty());

        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Widget"),
            type_name: "Widget".to_string(),
            collection_name: "widget".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "widget".to_string(),
                ty: TypeRefIr::LocalType { type_index: 0 },
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };
        let metadata = recoverable_metadata_for_service_artifacts(
            "svc",
            &[unit],
            &[db],
            &[],
            RecoverableInputs {
                custom_restore_plans: std::slice::from_ref(&custom_plan),
                ..RecoverableInputs::default()
            },
        )
        .expect("declared custom restore plan should enter production metadata builder");

        assert!(metadata.custom_restore_plans.contains_key(custom_identity));
        assert_eq!(
            metadata.boundary_plans["db:Widget:field:widget"].custom_restore_plan_ref,
            Some(RecoverableCustomRestorePlanRef(custom_identity.to_string()))
        );
        assert_eq!(
            metadata.storage_lanes["db:Widget:field:widget"].lane,
            RecoverableStorageLane::RecoverableEnvelope
        );
    }

    #[test]
    fn undeclared_custom_restore_nominal_still_rejects_raw_graph() {
        let unit = local_type_unit(
            "Widget",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("stream".to_string(), stream_type())]),
            },
        );

        let error = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &TypeRefIr::LocalType { type_index: 0 },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("undeclared custom restore nominal must still validate raw closure");

        assert!(error.to_string().contains("Stream<T>"));
        assert!(error.to_string().contains("field stream"));
    }

    #[test]
    fn declared_native_adapter_allows_handle_and_sets_plan_ref() {
        let adapter_plan = native_adapter_plan("std.HostHandle");

        let (_, plan) = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[],
            &host_handle_type(),
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs {
                native_adapter_plans: std::slice::from_ref(&adapter_plan),
                ..RecoverableInputs::default()
            },
        )
        .expect("declared native adapter should validate durable state and allow native handle");

        assert_eq!(
            plan.native_adapter_plan_ref,
            Some(RecoverableNativeAdapterPlanRef(
                "native:std.HostHandle".to_string()
            ))
        );
        assert!(plan.custom_restore_plan_ref.is_none());

        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Widget"),
            type_name: "Widget".to_string(),
            collection_name: "widget".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "handle".to_string(),
                ty: host_handle_type(),
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };
        let metadata = recoverable_metadata_for_service_artifacts(
            "svc",
            &[],
            &[db],
            &[],
            RecoverableInputs {
                native_adapter_plans: std::slice::from_ref(&adapter_plan),
                ..RecoverableInputs::default()
            },
        )
        .expect("declared native adapter should enter production metadata builder");

        assert!(metadata
            .native_adapter_plans
            .contains_key("native:std.HostHandle"));
        assert_eq!(
            metadata.boundary_plans["db:Widget:field:handle"].native_adapter_plan_ref,
            Some(RecoverableNativeAdapterPlanRef(
                "native:std.HostHandle".to_string()
            ))
        );
        assert_eq!(
            metadata.storage_lanes["db:Widget:field:handle"].lane,
            RecoverableStorageLane::RecoverableEnvelope
        );
    }

    #[test]
    fn explicit_slot_rejects_map_number_key() {
        let error = recoverable_boundary_plan_for_type(
            "slot:test",
            "",
            &[],
            &map_type(TypeRefIr::native("number"), TypeRefIr::native("string")),
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("recoverable map keys should reject non-string primitive keys");

        assert!(error.to_string().contains("map key"));
        assert!(error.to_string().contains("must be string"));
    }

    #[test]
    fn explicit_slot_rejects_map_key_non_string_nominal_representation() {
        let unit = local_type_unit(
            "NumericKey",
            TypeDescriptorIr::Alias {
                target: TypeRefIr::native("number"),
            },
        );

        let error = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &map_type(
                TypeRefIr::LocalType { type_index: 0 },
                TypeRefIr::native("string"),
            ),
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect_err("recoverable map key nominal representation must be over string");

        assert!(error.to_string().contains("map key"));
        assert!(error.to_string().contains("number"));
    }

    #[test]
    fn explicit_slot_accepts_local_alias_string_map_key() {
        let unit = local_type_unit(
            "StringKey",
            TypeDescriptorIr::Alias {
                target: TypeRefIr::native("string"),
            },
        );

        recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[unit],
            &map_type(
                TypeRefIr::LocalType { type_index: 0 },
                TypeRefIr::native("string"),
            ),
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("recoverable map key should allow local alias over string");
    }

    #[test]
    fn explicit_slot_accepts_package_alias_string_map_key() {
        let (package_source, key_ty) = package_source_with_exported_string_alias();

        recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[],
            &map_type(key_ty, TypeRefIr::native("string")),
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs {
                package_sources: &[package_source],
                ..RecoverableInputs::default()
            },
        )
        .expect("recoverable map key should allow package alias over string");
    }

    #[test]
    fn package_exported_descriptor_collects_package_local_identity_refs() {
        let mut file = FileIrUnit::empty("pkg.data", "hash");
        file.file_ir_identity = "file:pkg.data".to_string();
        file.type_table.push(TypeDeclIr {
            name: "PkgParent".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "child".to_string(),
                    TypeRefIr::LocalType { type_index: 1 },
                )]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.type_table.push(TypeDeclIr {
            name: "PkgChild".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("label".to_string(), TypeRefIr::native("string"))]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "PkgParent".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PkgParent".to_string(),
                source_span: None,
            },
        );
        file.declarations.types.insert(
            "PkgChild".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "PkgChild".to_string(),
                source_span: None,
            },
        );

        let mut unit = PackageUnit::empty("pkg.example", "0.1.0", "build:pkg", "abi:pkg");
        unit.implementation_links.types.insert(
            "PkgParent".to_string(),
            TypeExport {
                file: FileIrRef::new("file:pkg.data", "pkg.data"),
                type_index: 0,
                symbol: "PkgParent".to_string(),
                descriptor: None,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        let package_source = RecoverablePackageTypeSource {
            package_id: "pkg.example".to_string(),
            dependency_refs: vec!["pkg_dep".to_string()],
            unit,
            file_ir_units: vec![file],
        };
        let package_ty = TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "pkg_dep".to_string(),
                },
                symbol_path: "PkgParent".to_string(),
                abi_expectation: None,
            },
        };

        let (_, plan) = recoverable_boundary_plan_for_type(
            "slot:test",
            "app",
            &[],
            &package_ty,
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs {
                package_sources: &[package_source],
                ..RecoverableInputs::default()
            },
        )
        .expect("package local nested descriptor should collect expected type identities");

        let field_refs = plan
            .expected_type
            .field_refs
            .iter()
            .map(|field_ref| field_ref.0.as_str())
            .collect::<Vec<_>>();
        assert!(field_refs
            .iter()
            .any(|field_ref| field_ref.contains("type:package:pkg.example:PkgParent:child")));
        assert!(field_refs.iter().any(|field_ref| {
            field_ref.contains("type:source:package:pkg.example:module:pkg.data:PkgChild:label")
        }));
        assert!(!field_refs
            .iter()
            .any(|field_ref| field_ref.contains("module:app") && field_ref.contains(":label")));
    }

    #[test]
    fn package_exported_interface_any_interface_collects_method_refs_and_signature_facts() {
        let (package_source, interface_ty) =
            package_source_with_exported_interface(TypeRefIr::native("string"));
        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Run"),
            type_name: "Run".to_string(),
            collection_name: "run".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "provider".to_string(),
                ty: TypeRefIr::AnyInterface {
                    interface: InterfaceInstantiationRef {
                        interface_abi_id: type_ref_abi_key(&interface_ty),
                        canonical_type_args: Vec::new(),
                    },
                },
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };

        let metadata = recoverable_metadata_for_service_artifacts(
            "svc",
            &[],
            &[db],
            &[],
            RecoverableInputs {
                package_sources: &[package_source],
                ..RecoverableInputs::default()
            },
        )
        .expect("package-exported interface should resolve through package implementation links");
        let plan = &metadata.boundary_plans["db:Run:field:provider"].expected_type;

        assert!(!plan.interface_method_refs.is_empty());
        let method_ref = &plan.interface_method_refs[0];
        let method_fact = metadata
            .identity_tables
            .interface_methods
            .get(&method_ref.0)
            .expect("method identity fact should be recorded for package interface");
        assert_eq!(method_fact.method_name, "call");
        let signature = method_fact
            .signature
            .as_ref()
            .expect("package interface method should carry signature expected plan");
        assert_eq!(
            signature.root,
            RecoverableExpectedTypeRoot::TypeRef {
                ty: TypeRefIr::native("string")
            }
        );
    }

    #[test]
    fn package_exported_interface_method_refs_include_signature_shape() {
        let (package_string, interface_ty) =
            package_source_with_exported_interface(TypeRefIr::native("string"));
        let (package_number, _) =
            package_source_with_exported_interface(TypeRefIr::native("number"));
        let any_package_interface = TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: type_ref_abi_key(&interface_ty),
                canonical_type_args: Vec::new(),
            },
        };

        let (_, string_plan) = recoverable_boundary_plan_for_type(
            "slot:string",
            "app",
            &[],
            &any_package_interface,
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs {
                package_sources: &[package_string],
                ..RecoverableInputs::default()
            },
        )
        .expect("string package interface should build a recoverable plan");
        let (_, number_plan) = recoverable_boundary_plan_for_type(
            "slot:number",
            "app",
            &[],
            &any_package_interface,
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs {
                package_sources: &[package_number],
                ..RecoverableInputs::default()
            },
        )
        .expect("number package interface should build a recoverable plan");

        assert_ne!(
            string_plan.expected_type.interface_method_refs,
            number_plan.expected_type.interface_method_refs
        );
    }

    #[test]
    fn db_lane_selects_envelope_for_any_interface_and_nested_any_interface() {
        assert_eq!(
            db_storage_lane_for_type(&any_interface()),
            RecoverableStorageLane::RecoverableEnvelope
        );
        let nested = TypeRefIr::Record {
            fields: BTreeMap::from([
                ("provider".to_string(), any_interface()),
                ("label".to_string(), TypeRefIr::native("string")),
            ]),
        };
        assert_eq!(
            db_storage_lane_for_type(&nested),
            RecoverableStorageLane::RecoverableEnvelope
        );
    }

    #[test]
    fn db_lane_selects_schema_projectable_for_plain_data() {
        let plain = TypeRefIr::Record {
            fields: BTreeMap::from([
                ("label".to_string(), TypeRefIr::native("string")),
                (
                    "tags".to_string(),
                    TypeRefIr::Native {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::native("string")],
                    },
                ),
                (
                    "flag".to_string(),
                    TypeRefIr::Literal {
                        value: LiteralIr::Bool { value: true },
                    },
                ),
            ]),
        };

        assert_eq!(
            db_storage_lane_for_type(&plain),
            RecoverableStorageLane::SchemaProjectable
        );
    }

    #[test]
    fn projection_builder_collects_local_record_field_refs() {
        let mut unit = FileIrUnit::empty("app", "hash");
        unit.type_table.push(TypeDeclIr {
            name: "Settings".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("label".to_string(), TypeRefIr::native("string"))]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Settings"),
            type_name: "Settings".to_string(),
            collection_name: "setting".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "settings".to_string(),
                ty: TypeRefIr::LocalType { type_index: 0 },
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };

        let metadata = recoverable_metadata_for_service_artifacts(
            "svc",
            &[unit],
            &[db],
            &[],
            RecoverableInputs::default(),
        )
        .unwrap();
        let plan = &metadata.boundary_plans["db:Settings:field:settings"].expected_type;

        assert!(plan
            .field_refs
            .iter()
            .any(|field| field.0.contains(":label")));
        assert!(metadata
            .identity_tables
            .fields
            .values()
            .any(|field| field.field_name == "label"));
    }

    #[test]
    fn projection_builder_rejects_db_local_record_nested_function() {
        let unit = local_type_unit(
            "Settings",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("callback".to_string(), callback_type())]),
            },
        );
        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Settings"),
            type_name: "Settings".to_string(),
            collection_name: "setting".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "settings".to_string(),
                ty: TypeRefIr::LocalType { type_index: 0 },
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };

        let error = recoverable_metadata_for_service_artifacts(
            "svc",
            &[unit],
            &[db],
            &[],
            RecoverableInputs::default(),
        )
        .expect_err("DB local nominal closure should reject nested function");

        assert!(error.to_string().contains("db field Settings.settings"));
        assert!(error.to_string().contains("callback function type"));
        assert!(error.to_string().contains("field callback"));
    }

    #[test]
    fn projection_builder_rejects_db_map_number_key() {
        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Settings"),
            type_name: "Settings".to_string(),
            collection_name: "setting".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "settings".to_string(),
                ty: map_type(TypeRefIr::native("number"), TypeRefIr::native("string")),
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };

        let error = recoverable_metadata_for_service_artifacts(
            "svc",
            &[],
            &[db],
            &[],
            RecoverableInputs::default(),
        )
        .expect_err("DB recoverable map key should reject non-string primitive keys");

        assert!(error.to_string().contains("db field Settings.settings"));
        assert!(error.to_string().contains("map key"));
    }

    #[test]
    fn projection_builder_rejects_spawn_map_number_key() {
        let target = spawn_target_with_param(map_type(
            TypeRefIr::native("number"),
            TypeRefIr::native("string"),
        ));

        let error = recoverable_metadata_for_service_artifacts(
            "svc",
            &[],
            &[],
            &[target],
            RecoverableInputs::default(),
        )
        .expect_err("spawn recoverable map key should reject non-string primitive keys");

        assert!(error.to_string().contains("spawn payload"));
        assert!(error.to_string().contains("map key"));
    }

    #[test]
    fn projection_builder_rejects_package_exported_native_descriptor_without_adapter() {
        let (package_source, native_ty) = package_source_with_exported_native_descriptor();
        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Settings"),
            type_name: "Settings".to_string(),
            collection_name: "setting".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "handle".to_string(),
                ty: native_ty,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };

        let error = recoverable_metadata_for_service_artifacts(
            "svc",
            &[],
            &[db],
            &[],
            RecoverableInputs {
                package_sources: &[package_source],
                ..RecoverableInputs::default()
            },
        )
        .expect_err("package exported native descriptor should require native adapter plan");

        assert!(error.to_string().contains("db field Settings.handle"));
        assert!(error
            .to_string()
            .contains("requires RecoverableNativeAdapterPlan"));
    }

    #[test]
    fn metadata_type_policy_rejects_custom_durable_local_record_closure() {
        let unit = local_type_unit(
            "RestoreState",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("stream".to_string(), stream_type())]),
            },
        );
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.custom_restore_plans.insert(
            "restore:state".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Widget".to_string(),
                restore_schema_version: "1".to_string(),
                durable_state_type_plan: RecoverableExpectedTypePlan {
                    root: RecoverableExpectedTypeRoot::TypeRef {
                        ty: TypeRefIr::LocalType { type_index: 0 },
                    },
                    root_type_identity_ref: None,
                    runtime_carrier_check_required: false,
                    interface_projection_refs: Vec::new(),
                    interface_method_refs: Vec::new(),
                    field_refs: Vec::new(),
                    union_branch_refs: Vec::new(),
                },
                encode_hook_id: "restore:state.encode".to_string(),
                decode_hook_id: "restore:state.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_recoverable_metadata_type_policy(&metadata, &[unit])
            .expect_err("custom durable state plan must validate descriptor closure");

        assert!(error
            .to_string()
            .contains("custom restore plan restore:state"));
        assert!(error.to_string().contains("Stream<T>"));
        assert!(error.to_string().contains("field stream"));
    }

    #[test]
    fn metadata_type_policy_rejects_unresolved_custom_durable_type_identity_ref() {
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.custom_restore_plans.insert(
            "restore:state".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Widget".to_string(),
                restore_schema_version: "1".to_string(),
                durable_state_type_plan: type_identity_expected_plan(RecoverableTypeIdentityRef(
                    "type:missing".to_string(),
                )),
                encode_hook_id: "restore:state.encode".to_string(),
                decode_hook_id: "restore:state.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_recoverable_metadata_type_policy(&metadata, &[])
            .expect_err("unresolved durable type identity must fail closed");

        assert!(error
            .to_string()
            .contains("custom restore plan restore:state"));
        assert!(error.to_string().contains("cannot be resolved"));
    }

    #[test]
    fn metadata_type_policy_rejects_durable_type_ref_native_without_adapter() {
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.custom_restore_plans.insert(
            "restore:state".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Widget".to_string(),
                restore_schema_version: "1".to_string(),
                durable_state_type_plan: RecoverableExpectedTypePlan {
                    root: RecoverableExpectedTypeRoot::TypeRef {
                        ty: host_handle_type(),
                    },
                    root_type_identity_ref: None,
                    runtime_carrier_check_required: false,
                    interface_projection_refs: Vec::new(),
                    interface_method_refs: Vec::new(),
                    field_refs: Vec::new(),
                    union_branch_refs: Vec::new(),
                },
                encode_hook_id: "restore:state.encode".to_string(),
                decode_hook_id: "restore:state.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_recoverable_metadata_type_policy(&metadata, &[])
            .expect_err("durable native state without adapter must fail closed");

        assert!(error
            .to_string()
            .contains("custom restore plan restore:state"));
        assert!(error.to_string().contains("std.HostHandle"));
        assert!(error
            .to_string()
            .contains("requires RecoverableNativeAdapterPlan"));
    }

    #[test]
    fn metadata_type_policy_accepts_durable_type_ref_native_with_adapter() {
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.custom_restore_plans.insert(
            "restore:state".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Widget".to_string(),
                restore_schema_version: "1".to_string(),
                durable_state_type_plan: RecoverableExpectedTypePlan {
                    root: RecoverableExpectedTypeRoot::TypeRef {
                        ty: host_handle_type(),
                    },
                    root_type_identity_ref: None,
                    runtime_carrier_check_required: false,
                    interface_projection_refs: Vec::new(),
                    interface_method_refs: Vec::new(),
                    field_refs: Vec::new(),
                    union_branch_refs: Vec::new(),
                },
                encode_hook_id: "restore:state.encode".to_string(),
                decode_hook_id: "restore:state.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );
        metadata.native_adapter_plans.insert(
            "native:host".to_string(),
            native_adapter_plan("std.HostHandle"),
        );

        validate_recoverable_metadata_type_policy(&metadata, &[])
            .expect("durable native state should be accepted when adapter metadata exists");
    }

    #[test]
    fn metadata_type_policy_rejects_durable_type_identity_ref_native_without_adapter() {
        let state_ty = host_handle_type();
        let state_identity_ref = structural_type_identity_ref(&state_ty);
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.identity_tables.types.insert(
            state_identity_ref.0.clone(),
            RecoverableTypeIdentityFact {
                ty: state_ty,
                abi_type_id: Some(state_identity_ref.0.clone()),
                contract_revision: None,
            },
        );
        metadata.custom_restore_plans.insert(
            "restore:state".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Widget".to_string(),
                restore_schema_version: "1".to_string(),
                durable_state_type_plan: type_identity_expected_plan(state_identity_ref),
                encode_hook_id: "restore:state.encode".to_string(),
                decode_hook_id: "restore:state.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_recoverable_metadata_type_policy(&metadata, &[])
            .expect_err("durable TypeIdentityRef native without adapter must fail closed");

        assert!(error
            .to_string()
            .contains("custom restore plan restore:state"));
        assert!(error.to_string().contains("std.HostHandle"));
    }

    #[test]
    fn metadata_type_policy_rejects_durable_map_number_key() {
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.custom_restore_plans.insert(
            "restore:state".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Widget".to_string(),
                restore_schema_version: "1".to_string(),
                durable_state_type_plan: RecoverableExpectedTypePlan {
                    root: RecoverableExpectedTypeRoot::TypeRef {
                        ty: map_type(TypeRefIr::native("number"), TypeRefIr::native("string")),
                    },
                    root_type_identity_ref: None,
                    runtime_carrier_check_required: false,
                    interface_projection_refs: Vec::new(),
                    interface_method_refs: Vec::new(),
                    field_refs: Vec::new(),
                    union_branch_refs: Vec::new(),
                },
                encode_hook_id: "restore:state.encode".to_string(),
                decode_hook_id: "restore:state.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_recoverable_metadata_type_policy(&metadata, &[])
            .expect_err("durable map key should reject non-string primitive keys");

        assert!(error
            .to_string()
            .contains("custom restore plan restore:state"));
        assert!(error.to_string().contains("map key"));
    }

    #[test]
    fn metadata_type_policy_rejects_native_durable_type_identity_ref_invalid_closure() {
        let unit = local_type_unit(
            "NativeState",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("stream".to_string(), stream_type())]),
            },
        );
        let state_ty = TypeRefIr::LocalType { type_index: 0 };
        let state_identity_ref = structural_type_identity_ref(&state_ty);
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.identity_tables.types.insert(
            state_identity_ref.0.clone(),
            RecoverableTypeIdentityFact {
                ty: state_ty,
                abi_type_id: Some(state_identity_ref.0.clone()),
                contract_revision: None,
            },
        );
        metadata.native_adapter_plans.insert(
            "native:state".to_string(),
            RecoverableNativeAdapterPlan {
                adapter_identity: "adapter:state".to_string(),
                adapter_schema_version: "1".to_string(),
                native_type_identity: "native:State".to_string(),
                durable_state_type_plan: type_identity_expected_plan(state_identity_ref),
                encode_hook_id: "adapter:state.encode".to_string(),
                decode_hook_id: "adapter:state.decode".to_string(),
                owner: RecoverableNativeAdapterOwner {
                    service_identity: "svc".to_string(),
                },
                schema_compatibility: RecoverableAdapterSchemaCompatibility::Exact,
            },
        );

        let error = validate_recoverable_metadata_type_policy(&metadata, &[unit])
            .expect_err("resolved durable type identity must validate closure");

        assert!(error
            .to_string()
            .contains("native adapter plan native:state"));
        assert!(error.to_string().contains("Stream<T>"));
        assert!(error.to_string().contains("field stream"));
    }

    #[test]
    fn metadata_type_policy_rejects_durable_type_identity_ref_to_package_symbol_closure() {
        let (package_source, package_ty) = package_source_with_exported_invalid_type();
        let package_identity_ref = structural_type_identity_ref(&package_ty);
        let mut metadata = RecoverableArtifactMetadata::default();
        metadata.identity_tables.types.insert(
            package_identity_ref.0.clone(),
            RecoverableTypeIdentityFact {
                ty: package_ty,
                abi_type_id: Some(package_identity_ref.0.clone()),
                contract_revision: None,
            },
        );
        metadata.custom_restore_plans.insert(
            "restore:pkg-state".to_string(),
            RecoverableCustomRestorePlan {
                concrete_type_identity: "type:Widget".to_string(),
                restore_schema_version: "1".to_string(),
                durable_state_type_plan: type_identity_expected_plan(package_identity_ref),
                encode_hook_id: "restore:pkg-state.encode".to_string(),
                decode_hook_id: "restore:pkg-state.decode".to_string(),
                restore_capability: RecoverableRestoreCapability::Exact,
            },
        );

        let error = validate_recoverable_metadata_type_policy_with_packages(
            &metadata,
            &[],
            &[package_source],
        )
        .expect_err("durable package symbol identity should validate package descriptor");

        assert!(error
            .to_string()
            .contains("custom restore plan restore:pkg-state"));
        assert!(error.to_string().contains("Stream<T>"));
        assert!(error.to_string().contains("field stream"));
    }

    #[test]
    fn projection_builder_collects_interface_method_refs_when_decl_available() {
        let mut unit = FileIrUnit::empty("app", "hash");
        unit.declarations.interfaces.insert(
            "ToolProvider".to_string(),
            InterfaceDeclIr {
                name: "ToolProvider".to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: "call".to_string(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: TypeRefIr::native("string"),
                    is_native: false,
                    is_provider: true,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );
        let db = DbMetadataIr {
            module_path: "app".to_string(),
            source_role: "service".to_string(),
            package_id: None,
            package_version: None,
            file_ir_identity: Some("file:app".to_string()),
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("Run"),
            type_name: "Run".to_string(),
            collection_name: "run".to_string(),
            key: None,
            fields: vec![DbObjectFieldIr {
                name: "provider".to_string(),
                ty: TypeRefIr::AnyInterface {
                    interface: InterfaceInstantiationRef {
                        interface_abi_id: "ToolProvider".to_string(),
                        canonical_type_args: vec![TypeRefIr::native("string")],
                    },
                },
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
        };

        let metadata = recoverable_metadata_for_service_artifacts(
            "svc",
            &[unit],
            &[db],
            &[],
            RecoverableInputs::default(),
        )
        .unwrap();
        let plan = &metadata.boundary_plans["db:Run:field:provider"].expected_type;

        let expected_method_id = recoverable_interface_method_abi_id(
            &InterfaceInstantiationRef {
                interface_abi_id: "ToolProvider".to_string(),
                canonical_type_args: vec![TypeRefIr::native("string")],
            },
            &InterfaceOperationIr {
                name: "call".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRefIr::native("string"),
                is_native: false,
                is_provider: true,
                is_static: false,
                implicit_self: None,
            },
        );
        assert!(plan
            .interface_method_refs
            .iter()
            .any(|method| method.0 == expected_method_id));
        assert!(metadata
            .identity_tables
            .interface_methods
            .values()
            .any(|method| method.method_name == "call"));
    }

    #[test]
    fn recoverable_interface_method_refs_include_signature_shape() {
        let interface = InterfaceInstantiationRef {
            interface_abi_id: "ToolProvider".to_string(),
            canonical_type_args: vec![TypeRefIr::native("string")],
        };
        let string_operation = InterfaceOperationIr {
            name: "call".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::native("string"),
            is_native: false,
            is_provider: true,
            is_static: false,
            implicit_self: None,
        };
        let number_operation = InterfaceOperationIr {
            name: "call".to_string(),
            type_params: Vec::new(),
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::native("number"),
            }],
            return_type: TypeRefIr::native("number"),
            is_native: false,
            is_provider: true,
            is_static: false,
            implicit_self: None,
        };

        assert_ne!(
            recoverable_interface_method_abi_id(&interface, &string_operation),
            recoverable_interface_method_abi_id(&interface, &number_operation)
        );

        let mut string_unit = FileIrUnit::empty("app", "hash");
        string_unit.declarations.interfaces.insert(
            "ToolProvider".to_string(),
            InterfaceDeclIr {
                name: "ToolProvider".to_string(),
                type_params: Vec::new(),
                operations: vec![string_operation],
                source_span: None,
            },
        );
        let mut number_unit = FileIrUnit::empty("app", "hash");
        number_unit.declarations.interfaces.insert(
            "ToolProvider".to_string(),
            InterfaceDeclIr {
                name: "ToolProvider".to_string(),
                type_params: Vec::new(),
                operations: vec![number_operation],
                source_span: None,
            },
        );

        let (_, string_plan) = recoverable_boundary_plan_for_type(
            "slot:string",
            "app",
            &[string_unit],
            &TypeRefIr::AnyInterface {
                interface: interface.clone(),
            },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("string method interface should build a recoverable plan");
        let (_, number_plan) = recoverable_boundary_plan_for_type(
            "slot:number",
            "app",
            &[number_unit],
            &TypeRefIr::AnyInterface { interface },
            RecoverableBoundaryKind::ServicePayload,
            RecoverableTrustBoundary::OwnerInternal,
            true,
            RecoverableInputs::default(),
        )
        .expect("number method interface should build a recoverable plan");

        assert_ne!(
            string_plan.expected_type.interface_method_refs,
            number_plan.expected_type.interface_method_refs
        );
    }

    #[test]
    fn envelope_lane_rejects_nested_projection_path() {
        let envelope_fields = BTreeSet::from(["settings"]);

        let error = validate_db_recoverable_envelope_field_path(
            "db projection on AgentRun",
            &FieldPathIr {
                text: "settings.label".to_string(),
                segments: vec!["settings".to_string(), "label".to_string()],
            },
            &envelope_fields,
        )
        .expect_err("nested envelope projection should fail");

        assert!(error.contains("cannot project through recoverable envelope field"));
    }
}
