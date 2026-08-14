use std::collections::BTreeMap;

use skiff_artifact_model::{
    classify_value_lifecycle, native_value_lifecycle_registry, BytecodePoolEntry,
    CallableRegistryTypeExpression, ContractTypeRef, InterfaceInstantiationRef, LiteralIr,
    NativeResourceDropPlan, NativeValueAdapterRole, NativeValueDropPlan, NativeValueEmbedding,
    NativeValueLifecycleConcrete, PackageLocalAbiSymbol, PackageRefIr, PackageSchemaTypeRecord,
    PositionalTypeEnvironment, PrivilegedAffineCompositeIdentity, ResolvedPackageValueType,
    ResourceDropPlan, TypeDescriptorIr, TypeRefIr, ValueDropPlan, ValueLifecycleFactResolver,
    ValueLifecyclePolicyBudget, ValueLifecycleResolverError, ValueTransferPlan,
};
use skiff_runtime_linked_bytecode::{
    LinkedResourceDropPlan, LinkedShapeField, LinkedValueDropPlan, LinkedValueTransferPlan,
    ShapeIndex, SpecializationKey,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::{interner::TypeLinker, normalize_type, substitution::substitute_type};

impl TypeLinker<'_> {
    /// Eliminates a plan expression owned by a fingerprinted callable
    /// registry row. Ordinary `FromType` remains unavailable everywhere else;
    /// the registry row is the authority for this exact signature position.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::bytecode) fn link_registry_plan_for_type_at(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        declared: &ValueTransferPlan,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        let ValueTransferPlan::FromType { ty } = declared else {
            return self.link_plan_for_type_at(
                package,
                specialization,
                substitutions,
                declared,
                concrete_type,
                location,
            );
        };
        let declared_type = substitute_type(ty, substitutions, &location)?;
        let declared_type = normalize_type(self.deployment(), package, &declared_type, &location)?;
        if declared_type != *concrete_type {
            return Err(obligation_error(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location,
                "registry FromType plan does not name its exact normalized signature type"
                    .to_string(),
            ));
        }
        self.plan_for_concrete_type_at(
            package,
            specialization,
            substitutions,
            concrete_type,
            location,
        )
    }

    /// Links a lifecycle plan at an exact artifact position. Recursive shape
    /// references are never accepted without the current package and
    /// specialization, so a linked `ShapeIndex` cannot be manufactured from a
    /// type name or a registry lifecycle alone.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::bytecode) fn link_plan_for_type_at(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        declared: &ValueTransferPlan,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        match declared {
            ValueTransferPlan::FromType { ty } => {
                if privileged_identity_for_type(concrete_type).is_none() {
                    return Err(BytecodeLinkError::ImplementationUnavailable {
                        obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location,
                    });
                }
                let declared_type = substitute_type(ty, substitutions, &location)?;
                let declared_type =
                    normalize_type(self.deployment(), package, &declared_type, &location)?;
                if declared_type != *concrete_type {
                    return Err(obligation_error(
                        BytecodeLinkObligation::FrameAndValueTransferPlan,
                        location,
                        "FromType plan does not name its exact normalized placement type"
                            .to_string(),
                    ));
                }
                self.plan_for_concrete_type_at(
                    package,
                    specialization,
                    substitutions,
                    concrete_type,
                    location,
                )
            }
            ValueTransferPlan::MoveOnly {
                drop: ValueDropPlan::RecursiveShape { shape_ref },
            } => {
                let shape = self.intern_pool_shape(
                    package,
                    specialization,
                    *shape_ref,
                    substitutions,
                    location.clone(),
                )?;
                self.require_privileged_root_shape(shape, concrete_type, location)?;
                Ok(LinkedValueTransferPlan::MoveOnly {
                    drop: LinkedValueDropPlan::RecursiveShape { shape },
                })
            }
            concrete => self.link_transfer_plan(concrete, substitutions, location),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::bytecode) fn plan_for_concrete_type_at(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        ty: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        let Some(identity) = privileged_identity_for_type(ty) else {
            return self.plan_for_concrete_type(ty, location);
        };
        let shape = self.find_exact_privileged_shape(
            package,
            specialization,
            substitutions,
            identity,
            ty,
            location.clone(),
        )?;
        self.require_privileged_root_shape(shape, ty, location)?;
        Ok(LinkedValueTransferPlan::MoveOnly {
            drop: LinkedValueDropPlan::RecursiveShape { shape },
        })
    }

    fn find_exact_privileged_shape(
        &mut self,
        package: &HydratedBytecodePackage,
        specialization: &SpecializationKey,
        substitutions: &BTreeMap<String, TypeRefIr>,
        identity: PrivilegedAffineCompositeIdentity,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<ShapeIndex, BytecodeLinkError> {
        let bytecode = package.bytecode().ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                "privileged affine value owner has no admitted bytecode shape pool".to_string(),
            )
        })?;
        let matches = bytecode
            .view()
            .pools()
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match entry {
                BytecodePoolEntry::ShapeRef { shape }
                    if shape.privileged_affine_composite == Some(identity) =>
                {
                    u32::try_from(index).ok()
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [artifact_shape] = matches.as_slice() else {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
                format!(
                    "privileged affine identity {identity:?} requires exactly one explicit admitted shape, found {}",
                    matches.len()
                ),
            ));
        };
        let shape = self.intern_pool_shape(
            package,
            specialization,
            *artifact_shape,
            substitutions,
            location.clone(),
        )?;
        self.require_privileged_root_shape(shape, concrete_type, location)?;
        Ok(shape)
    }

    pub(in crate::bytecode) fn validate_privileged_shape_authority(
        &self,
        identity: Option<PrivilegedAffineCompositeIdentity>,
        nominal_type: skiff_runtime_linked_bytecode::TypeIndex,
        fields: &[LinkedShapeField],
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        let nominal = self.linked_type_ref(nominal_type).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                "linked privileged shape nominal type is absent".to_string(),
            )
        })?;
        let registry = native_value_lifecycle_registry();
        let symbol_schema = match nominal {
            TypeRefIr::PackageSymbol { symbol } => {
                registry.privileged_affine_composite_for_symbol(symbol)
            }
            _ => None,
        };
        let Some(identity) = identity else {
            if symbol_schema.is_some() {
                return Err(obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                    "privileged affine nominal type lacks explicit linked shape authority"
                        .to_string(),
                ));
            }
            return Ok(());
        };
        let schema = registry
            .privileged_affine_composite(identity)
            .ok_or_else(|| {
                obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!("linked privileged affine identity {identity:?} is not pinned"),
                )
            })?;
        if symbol_schema.map(|row| row.identity) != Some(identity) {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                "linked privileged affine identity does not match its exact nominal symbol"
                    .to_string(),
            ));
        }
        if fields.len() != schema.fields.len() {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
                "privileged affine linked field count differs from the pinned schema".to_string(),
            ));
        }
        for (ordinal, (field, expected)) in fields.iter().zip(&schema.fields).enumerate() {
            let actual_type = self.linked_type_ref(field.ty()).ok_or_else(|| {
                obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!("privileged affine field {ordinal} type is absent"),
                )
            })?;
            let expected_plan =
                bridge_nonrecursive_lifecycle(&expected.lifecycle).ok_or_else(|| {
                    obligation_error(
                        BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                        location.clone(),
                        format!("privileged affine field {ordinal} has a recursive registry plan"),
                    )
                })?;
            if field.name() != expected.name
                || !matches_registry_type(&expected.ty, actual_type)
                || field.plan() != &expected_plan
            {
                return Err(obligation_error(
                    BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location.clone(),
                    format!(
                        "privileged affine linked field {ordinal} differs from its exact pinned name/type/lifecycle"
                    ),
                ));
            }
        }
        Ok(())
    }

    fn require_privileged_root_shape(
        &self,
        shape: ShapeIndex,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        let row = self.shape(shape).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                format!("privileged affine shape {} is absent", shape.get()),
            )
        })?;
        let nominal = self.linked_type_ref(row.nominal_type()).ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location.clone(),
                "privileged affine shape nominal type is absent".to_string(),
            )
        })?;
        if nominal != concrete_type || row.privileged_affine_composite().is_none() {
            return Err(obligation_error(
                BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                location,
                "recursive root plan does not bind the exact privileged nominal shape".to_string(),
            ));
        }
        Ok(())
    }

    pub(in crate::bytecode) fn link_transfer_plan(
        &self,
        plan: &ValueTransferPlan,
        _substitutions: &BTreeMap<String, TypeRefIr>,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        match plan {
            ValueTransferPlan::SnapshotShare { drop } => {
                Ok(LinkedValueTransferPlan::SnapshotShare {
                    drop: self.link_value_drop(drop, location)?,
                })
            }
            ValueTransferPlan::MoveOnly { drop } => Ok(LinkedValueTransferPlan::MoveOnly {
                drop: self.link_value_drop(drop, location)?,
            }),
            ValueTransferPlan::AffineResource { drop } => {
                Ok(LinkedValueTransferPlan::AffineResource {
                    drop: self.link_resource_drop(drop, location)?,
                })
            }
            ValueTransferPlan::ExplicitCloneLease {
                clone_adapter,
                drop,
            } => Ok(LinkedValueTransferPlan::ExplicitCloneLease {
                clone_adapter: lifecycle_adapter(
                    &clone_adapter.binding_key,
                    NativeValueAdapterRole::CloneLease,
                    location.clone(),
                )?,
                drop: self.link_resource_drop(drop, location)?,
            }),
            ValueTransferPlan::FromType { .. } => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
                    location,
                })
            }
        }
    }

    pub(in crate::bytecode) fn plan_for_concrete_type(
        &self,
        ty: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        if is_canonical_sleep_duration_type(ty) {
            return Ok(LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            });
        }
        match ty {
            TypeRefIr::Record { .. } | TypeRefIr::Union { .. } | TypeRefIr::Nullable { .. } => {
                Ok(LinkedValueTransferPlan::SnapshotShare {
                    drop: LinkedValueDropPlan::SnapshotRelease,
                })
            }
            TypeRefIr::Literal { .. } => {
                let resolution = native_value_lifecycle_registry()
                    .lookup(&lifecycle_registry_type(ty))
                    .map_err(|error| {
                        obligation_error(
                            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                            location.clone(),
                            format!("concrete literal has no authoritative lifecycle: {error}"),
                        )
                    })?;
                link_native_lifecycle(resolution.lifecycle, location)
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let package_id = match &symbol.package {
                    PackageRefIr::PackageId { package_id } => package_id,
                    PackageRefIr::Dependency { .. } => {
                        return Err(obligation_error(
                            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                            location.clone(),
                            "package symbol retains an unresolved dependency alias".to_string(),
                        ));
                    }
                };
                let owner = self
                    .deployment()
                    .packages()
                    .values()
                    .find(|package| package.reference().package_id == *package_id)
                    .ok_or_else(|| {
                        obligation_error(
                            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                            location.clone(),
                            format!("package symbol owner {package_id:?} is absent"),
                        )
                    })?;
                let mut resolver = ValidationLifecycleResolver::new(self.deployment(), owner);
                let mut budget =
                    ValueLifecyclePolicyBudget::new(1_000, 1_000_000, 64).map_err(|error| {
                        obligation_error(
                            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                            location.clone(),
                            error.to_string(),
                        )
                    })?;
                let resolution = classify_value_lifecycle(
                    ty,
                    &PositionalTypeEnvironment::empty(),
                    &mut resolver,
                    &mut budget,
                )
                .map_err(|error| {
                    obligation_error(
                        BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                        location.clone(),
                        error.to_string(),
                    )
                })?;
                link_native_lifecycle(resolution.lifecycle, location)
            }
            TypeRefIr::Builtin { name, args }
                if (name == "Exception" || name == "CatchResult") && !args.is_empty() =>
            {
                Ok(LinkedValueTransferPlan::SnapshotShare {
                    drop: LinkedValueDropPlan::SnapshotRelease,
                })
            }
            TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
                self.plan_for_concrete_type(&args[0], location.clone())?;
                Ok(LinkedValueTransferPlan::SnapshotShare {
                    drop: LinkedValueDropPlan::SnapshotRelease,
                })
            }
            TypeRefIr::Builtin { name, args } if name == "Map" && args.len() == 2 => {
                self.plan_for_concrete_type(&args[0], location.clone())?;
                self.plan_for_concrete_type(&args[1], location.clone())?;
                Ok(LinkedValueTransferPlan::SnapshotShare {
                    drop: LinkedValueDropPlan::SnapshotRelease,
                })
            }
            _ => {
                let resolution = native_value_lifecycle_registry()
                    .lookup(ty)
                    .map_err(|error| {
                        obligation_error(
                            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                            location.clone(),
                            format!("concrete value has no authoritative lifecycle: {error}"),
                        )
                    })?;
                link_native_lifecycle(resolution.lifecycle, location)
            }
        }
    }

    /// Eliminates a constant-local `FromType` only after checking that it names
    /// the exact linked type and that the authoritative lifecycle is an
    /// ordinary snapshot. Frame-local `FromType` remains unsupported.
    pub(in crate::bytecode) fn link_constant_plan(
        &self,
        declared: &ValueTransferPlan,
        concrete_type: &TypeRefIr,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
        if is_canonical_sleep_duration_type(concrete_type) {
            let expected = LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            };
            let actual = match declared {
                ValueTransferPlan::FromType { ty } if is_canonical_sleep_duration_type(ty) => {
                    expected.clone()
                }
                concrete => self
                    .link_transfer_plan(concrete, &BTreeMap::new(), location.clone())
                    .map_err(|error| constant_plan_error(location.clone(), error.to_string()))?,
            };
            if actual != expected {
                return Err(constant_plan_error(
                    location,
                    "std.time.Duration frozen constant plan is not the exact trivial snapshot"
                        .to_string(),
                ));
            }
            return Ok(actual);
        }
        let registry_type = lifecycle_registry_type(concrete_type);
        let resolution = native_value_lifecycle_registry()
            .lookup(&registry_type)
            .map_err(|error| constant_plan_error(location.clone(), error.to_string()))?;
        if resolution.embedding != NativeValueEmbedding::Ordinary
            || !matches!(
                &resolution.lifecycle,
                NativeValueLifecycleConcrete::SnapshotShare { .. }
            )
        {
            return Err(constant_plan_error(
                location,
                "frozen constant type is not an Ordinary SnapshotShare value".to_string(),
            ));
        }
        let expected = link_native_lifecycle(resolution.lifecycle, location.clone())
            .map_err(|error| constant_plan_error(location.clone(), error.to_string()))?;
        let actual = match declared {
            ValueTransferPlan::FromType { ty } if ty == concrete_type => expected.clone(),
            ValueTransferPlan::FromType { .. } => {
                return Err(constant_plan_error(
                    location,
                    "constant FromType plan does not name its exact linked type".to_string(),
                ));
            }
            concrete => self
                .link_transfer_plan(concrete, &BTreeMap::new(), location.clone())
                .map_err(|error| constant_plan_error(location.clone(), error.to_string()))?,
        };
        if actual != expected {
            return Err(constant_plan_error(
                location,
                "constant plan differs from its authoritative concrete lifecycle".to_string(),
            ));
        }
        Ok(actual)
    }

    fn link_value_drop(
        &self,
        drop: &ValueDropPlan,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedValueDropPlan, BytecodeLinkError> {
        match drop {
            ValueDropPlan::Trivial => Ok(LinkedValueDropPlan::Trivial),
            ValueDropPlan::SnapshotRelease => Ok(LinkedValueDropPlan::SnapshotRelease),
            ValueDropPlan::NativeAdapter { adapter } => Ok(LinkedValueDropPlan::NativeAdapter {
                adapter: lifecycle_adapter(
                    &adapter.binding_key,
                    NativeValueAdapterRole::ValueDrop,
                    location,
                )?,
            }),
            ValueDropPlan::RecursiveShape { .. } => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                })
            }
        }
    }

    fn link_resource_drop(
        &self,
        drop: &ResourceDropPlan,
        location: BytecodeLinkLocation,
    ) -> Result<LinkedResourceDropPlan, BytecodeLinkError> {
        match drop {
            ResourceDropPlan::ResourceTableRelease => {
                Ok(LinkedResourceDropPlan::ResourceTableRelease)
            }
            ResourceDropPlan::NativeAdapter { adapter } => {
                Ok(LinkedResourceDropPlan::NativeAdapter {
                    adapter: lifecycle_adapter(
                        &adapter.binding_key,
                        NativeValueAdapterRole::ResourceDrop,
                        location,
                    )?,
                })
            }
            ResourceDropPlan::RecursiveShape { .. } => {
                Err(BytecodeLinkError::ImplementationUnavailable {
                    obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    location,
                })
            }
        }
    }
}

struct ValidationLifecycleResolver<'a> {
    deployment: &'a HydratedDeploymentBytecode,
    owner: &'a HydratedBytecodePackage,
}

impl<'a> ValidationLifecycleResolver<'a> {
    fn new(deployment: &'a HydratedDeploymentBytecode, owner: &'a HydratedBytecodePackage) -> Self {
        Self { deployment, owner }
    }
}

impl ValueLifecycleFactResolver for ValidationLifecycleResolver<'_> {
    fn resolve_package_symbol(
        &mut self,
        symbol: &skiff_artifact_model::PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        let owner = match &symbol.package {
            PackageRefIr::PackageId { package_id } => self
                .deployment
                .packages()
                .values()
                .find(|package| package.reference().package_id == *package_id)
                .ok_or_else(|| resolver_error("package owner absent"))?,
            PackageRefIr::Dependency { .. } => {
                return Err(resolver_error(
                    "package symbol retains an unresolved dependency alias",
                ));
            }
        };
        let resolved = owner
            .artifact()
            .package_local_abi
            .implementation_symbols
            .get(&symbol.symbol_path)
            .or_else(|| {
                owner
                    .artifact()
                    .package_local_abi
                    .public_symbols
                    .get(&symbol.symbol_path)
            })
            .ok_or_else(|| resolver_error("package symbol absent"))?;
        let PackageLocalAbiSymbol::Type {
            descriptor,
            type_params,
            ..
        } = resolved
        else {
            return Err(resolver_error("package symbol is not a type"));
        };
        let location = BytecodeLinkLocation::Package {
            package: Box::new(owner.reference().clone()),
        };
        let descriptor =
            normalize_resolved_descriptor(self.deployment, owner, descriptor, &location)
                .map_err(|error| resolver_error(error.to_string()))?;
        Ok(ResolvedPackageValueType {
            type_parameters: type_params.clone(),
            descriptor,
        })
    }

    fn resolve_package_schema(
        &mut self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &skiff_artifact_model::PackageSchemaTypeId,
    ) -> Result<PackageSchemaTypeRecord, ValueLifecycleResolverError> {
        self.owner
            .artifact()
            .bytecode_schema_records
            .get(package_schema_type_id)
            .filter(|record| {
                record.package_id == package_id && record.stable_schema_key == stable_schema_key
            })
            .cloned()
            .ok_or_else(|| resolver_error("schema record absent"))
    }

    fn validate_interface(
        &mut self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
            .map_err(|_| resolver_error("interface identity is not TypeRefIr"))?;
        let TypeRefIr::PackageSymbol { symbol } = identity else {
            return Err(resolver_error("interface identity is not PackageSymbol"));
        };
        self.resolve_package_symbol(&symbol).map(|_| ())
    }

    fn validate_contract_interface(
        &mut self,
        interface: &ContractTypeRef,
        arguments: &[ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError> {
        let ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } = interface
        else {
            return Err(resolver_error("contract interface is not PackageSchema"));
        };
        let record =
            self.resolve_package_schema(package_id, stable_schema_key, package_schema_type_id)?;
        if !matches!(
            record.canonical_descriptor.descriptor,
            skiff_artifact_model::ContractTypeDescriptor::CallbackInterface { .. }
        ) {
            return Err(resolver_error("contract interface is not callback"));
        }
        if record.canonical_descriptor.type_params.len() != arguments.len() {
            return Err(resolver_error("contract interface arity mismatch"));
        }
        Ok(())
    }
}

fn normalize_resolved_descriptor(
    deployment: &HydratedDeploymentBytecode,
    owner: &HydratedBytecodePackage,
    descriptor: &TypeDescriptorIr,
    location: &BytecodeLinkLocation,
) -> Result<TypeDescriptorIr, BytecodeLinkError> {
    let normalize_ty = |ty: &TypeRefIr| normalize_type(deployment, owner, ty, location);
    Ok(match descriptor {
        TypeDescriptorIr::Record { fields } => TypeDescriptorIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| Ok((name.clone(), normalize_ty(ty)?)))
                .collect::<Result<_, BytecodeLinkError>>()?,
        },
        TypeDescriptorIr::Representation { representation } => TypeDescriptorIr::Representation {
            representation: normalize_ty(representation)?,
        },
        TypeDescriptorIr::Union { branches } => TypeDescriptorIr::Union {
            branches: branches
                .iter()
                .map(|branch| match branch {
                    skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        Ok(skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal {
                            nominal_type: normalize_ty(nominal_type)?,
                        })
                    }
                    skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type,
                        discriminator_field,
                        discriminator_value,
                    } => Ok(
                        skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                            payload_type: normalize_ty(payload_type)?,
                            discriminator_field: discriminator_field.clone(),
                            discriminator_value: discriminator_value.clone(),
                        },
                    ),
                    skiff_artifact_model::NamedUnionBranchIr::Literal { value } => {
                        Ok(skiff_artifact_model::NamedUnionBranchIr::Literal {
                            value: value.clone(),
                        })
                    }
                })
                .collect::<Result<_, BytecodeLinkError>>()?,
        },
        TypeDescriptorIr::Alias { target } => TypeDescriptorIr::Alias {
            target: normalize_ty(target)?,
        },
        TypeDescriptorIr::Interface => TypeDescriptorIr::Interface,
    })
}

fn resolver_error(message: impl Into<String>) -> ValueLifecycleResolverError {
    ValueLifecycleResolverError {
        authority: "bytecodeLinker.hydratedValueLifecycle".to_string(),
        message: message.into(),
    }
}

fn is_canonical_sleep_duration_type(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            symbol.symbol_path == "std.time.Duration"
                && matches!(
                    &symbol.package,
                    PackageRefIr::PackageId { package_id }
                        if package_id == "skiff.run/std"
                )
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            arguments.is_empty()
                && matches!(
                    base,
                    skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { symbol }
                        if is_canonical_sleep_duration_type(&TypeRefIr::PackageSymbol { symbol: symbol.clone() })
                )
        }
        _ => false,
    }
}

fn lifecycle_registry_type(ty: &TypeRefIr) -> TypeRefIr {
    match ty {
        TypeRefIr::Literal { value } => TypeRefIr::builtin(match value {
            LiteralIr::Null => "null",
            LiteralIr::Bool { .. } => "bool",
            LiteralIr::Number { .. } => "number",
            LiteralIr::String { .. } => "string",
        }),
        _ => ty.clone(),
    }
}

fn privileged_identity_for_type(ty: &TypeRefIr) -> Option<PrivilegedAffineCompositeIdentity> {
    let TypeRefIr::PackageSymbol { symbol } = ty else {
        return None;
    };
    native_value_lifecycle_registry()
        .privileged_affine_composite_for_symbol(symbol)
        .map(|schema| schema.identity)
}

fn matches_registry_type(expected: &CallableRegistryTypeExpression, actual: &TypeRefIr) -> bool {
    match (expected, actual) {
        (
            CallableRegistryTypeExpression::Builtin { name, arguments },
            TypeRefIr::Builtin {
                name: actual_name,
                args,
            },
        ) => {
            name == actual_name
                && arguments.len() == args.len()
                && arguments
                    .iter()
                    .zip(args)
                    .all(|(expected, actual)| matches_registry_type(expected, actual))
        }
        (
            CallableRegistryTypeExpression::PackageSymbol {
                package_id,
                symbol_path,
            },
            TypeRefIr::PackageSymbol { symbol },
        ) => {
            symbol.symbol_path == *symbol_path
                && matches!(
                    &symbol.package,
                    PackageRefIr::PackageId {
                        package_id: actual_package_id,
                    } if actual_package_id == package_id
                )
        }
        _ => false,
    }
}

fn bridge_nonrecursive_lifecycle(
    lifecycle: &NativeValueLifecycleConcrete,
) -> Option<LinkedValueTransferPlan> {
    match lifecycle {
        NativeValueLifecycleConcrete::SnapshotShare { drop } => {
            Some(LinkedValueTransferPlan::SnapshotShare {
                drop: match drop {
                    NativeValueDropPlan::Trivial => LinkedValueDropPlan::Trivial,
                    NativeValueDropPlan::SnapshotRelease => LinkedValueDropPlan::SnapshotRelease,
                    NativeValueDropPlan::NativeAdapter { adapter } => {
                        LinkedValueDropPlan::NativeAdapter {
                            adapter: adapter.clone(),
                        }
                    }
                    NativeValueDropPlan::PrivilegedRecursiveShape => return None,
                },
            })
        }
        NativeValueLifecycleConcrete::MoveOnly { drop } => {
            Some(LinkedValueTransferPlan::MoveOnly {
                drop: match drop {
                    NativeValueDropPlan::Trivial => LinkedValueDropPlan::Trivial,
                    NativeValueDropPlan::SnapshotRelease => LinkedValueDropPlan::SnapshotRelease,
                    NativeValueDropPlan::NativeAdapter { adapter } => {
                        LinkedValueDropPlan::NativeAdapter {
                            adapter: adapter.clone(),
                        }
                    }
                    NativeValueDropPlan::PrivilegedRecursiveShape => return None,
                },
            })
        }
        NativeValueLifecycleConcrete::AffineResource { drop } => {
            Some(LinkedValueTransferPlan::AffineResource {
                drop: link_native_resource_drop(drop.clone()),
            })
        }
        NativeValueLifecycleConcrete::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => Some(LinkedValueTransferPlan::ExplicitCloneLease {
            clone_adapter: clone_adapter.clone(),
            drop: link_native_resource_drop(drop.clone()),
        }),
    }
}

fn constant_plan_error(location: BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation: BytecodeLinkObligation::ConstantInitializationPlan,
        location,
        detail,
    }
}

pub(super) fn type_metrics(
    ty: &TypeRefIr,
    location: &BytecodeLinkLocation,
) -> Result<(u64, u64), BytecodeLinkError> {
    let mut nodes = 0;
    let mut max_depth = 0;
    visit_type(ty, 1, &mut nodes, &mut max_depth, location)?;
    Ok((nodes, max_depth))
}

fn visit_type(
    ty: &TypeRefIr,
    depth: u64,
    nodes: &mut u64,
    max_depth: &mut u64,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    *nodes = nodes.checked_add(1).ok_or_else(|| {
        obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location.clone(),
            "arithmetic overflow while measuring a concrete type".to_string(),
        )
    })?;
    *max_depth = (*max_depth).max(depth);
    match ty {
        TypeRefIr::Builtin { args, .. } => visit_types(args, depth, nodes, max_depth, location)?,
        TypeRefIr::AppliedNominal { arguments, .. } => {
            visit_types(arguments, depth, nodes, max_depth, location)?;
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                visit_type(field, depth + 1, nodes, max_depth, location)?;
            }
        }
        TypeRefIr::Union { items } => visit_types(items, depth, nodes, max_depth, location)?,
        TypeRefIr::Nullable { inner } => {
            visit_type(inner, depth + 1, nodes, max_depth, location)?;
        }
        TypeRefIr::AnyInterface { interface } => visit_types(
            &interface.canonical_type_args,
            depth,
            nodes,
            max_depth,
            location,
        )?,
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                visit_type(&parameter.ty, depth + 1, nodes, max_depth, location)?;
            }
            visit_type(return_type, depth + 1, nodes, max_depth, location)?;
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

fn visit_types(
    types: &[TypeRefIr],
    depth: u64,
    nodes: &mut u64,
    max_depth: &mut u64,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    for ty in types {
        visit_type(ty, depth + 1, nodes, max_depth, location)?;
    }
    Ok(())
}

fn lifecycle_adapter(
    binding_key: &str,
    expected_role: NativeValueAdapterRole,
    location: BytecodeLinkLocation,
) -> Result<skiff_artifact_model::NativeValueLifecycleAdapter, BytecodeLinkError> {
    native_value_lifecycle_registry()
        .adapter(binding_key)
        .filter(|adapter| adapter.role == expected_role)
        .cloned()
        .ok_or_else(|| {
            obligation_error(
                BytecodeLinkObligation::FrameAndValueTransferPlan,
                location,
                format!(
                    "lifecycle adapter {binding_key:?} is absent or has the wrong authoritative role"
                ),
            )
        })
}

fn link_native_lifecycle(
    lifecycle: NativeValueLifecycleConcrete,
    location: BytecodeLinkLocation,
) -> Result<LinkedValueTransferPlan, BytecodeLinkError> {
    bridge_nonrecursive_lifecycle(&lifecycle).ok_or_else(|| {
        obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "privileged recursive lifecycle requires an exact explicit artifact shape".to_string(),
        )
    })
}

fn link_native_resource_drop(drop: NativeResourceDropPlan) -> LinkedResourceDropPlan {
    match drop {
        NativeResourceDropPlan::ResourceTableRelease => {
            LinkedResourceDropPlan::ResourceTableRelease
        }
        NativeResourceDropPlan::NativeAdapter { adapter } => {
            LinkedResourceDropPlan::NativeAdapter { adapter }
        }
    }
}

fn obligation_error(
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
