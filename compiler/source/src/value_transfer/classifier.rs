use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    LiteralIr, NamedUnionBranchIr, NativeValueEmbedding, NominalTypeRefBaseIr, PackageRefIr,
    PackageSymbolRef, ServiceSymbolRef, TypeDescriptorIr, TypeRefIr, ValueDropPlan,
    ValueTransferPlan, ValueTransferPlanKind,
};
use skiff_compiler_core::prelude_registry::canonical_file_ir_builtin;

use super::{
    SourceValueTransferError, SourceValueTransferFacts, SourceValueTransferNominalFact,
    SourceValueTransferNominalId, SourceValueTransferNominalSemantics,
    SourceValueTransferPackageRef, SourceValueTransferPosition,
};

#[derive(Debug, Clone)]
pub(super) enum PlanProof {
    Concrete(ValueTransferPlan),
    Deferred {
        known_kind: Option<ValueTransferPlanKind>,
    },
}

#[derive(Debug, Clone)]
pub(super) struct Classification {
    pub(super) ty: TypeRefIr,
    pub(super) proof: PlanProof,
    pub(super) embedding: Option<NativeValueEmbedding>,
}

impl Classification {
    pub(super) fn concrete(
        ty: TypeRefIr,
        plan: ValueTransferPlan,
        embedding: NativeValueEmbedding,
    ) -> Self {
        debug_assert!(plan.concrete_kind().is_some());
        Self {
            ty,
            proof: PlanProof::Concrete(plan),
            embedding: Some(embedding),
        }
    }

    pub(super) fn deferred(
        ty: TypeRefIr,
        known_kind: Option<ValueTransferPlanKind>,
        embedding: Option<NativeValueEmbedding>,
    ) -> Self {
        Self {
            ty,
            proof: PlanProof::Deferred { known_kind },
            embedding,
        }
    }

    pub(super) fn is_deferred(&self) -> bool {
        matches!(self.proof, PlanProof::Deferred { .. })
    }

    pub(super) fn known_kind(&self) -> Option<ValueTransferPlanKind> {
        match &self.proof {
            PlanProof::Concrete(plan) => plan.concrete_kind(),
            PlanProof::Deferred { known_kind } => *known_kind,
        }
    }

    pub(super) fn concrete_plan(&self) -> Option<&ValueTransferPlan> {
        match &self.proof {
            PlanProof::Concrete(plan) => Some(plan),
            PlanProof::Deferred { .. } => None,
        }
    }

    fn into_plan(self) -> ValueTransferPlan {
        match self.proof {
            PlanProof::Concrete(plan) => plan,
            PlanProof::Deferred { .. } => ValueTransferPlan::FromType { ty: self.ty },
        }
    }
}

pub(super) struct Classifier<'facts, 'registry> {
    pub(super) facts: &'facts SourceValueTransferFacts,
    pub(super) registry: &'registry skiff_artifact_model::NativeValueLifecycleRegistry,
    binders: BTreeSet<String>,
    active_nominals: BTreeSet<SourceValueTransferNominalId>,
    pub(super) native_depth: usize,
}

impl<'facts, 'registry> Classifier<'facts, 'registry> {
    pub(super) fn new(
        facts: &'facts SourceValueTransferFacts,
        registry: &'registry skiff_artifact_model::NativeValueLifecycleRegistry,
        binders: &[String],
    ) -> Result<Self, SourceValueTransferError> {
        let mut exact_binders = BTreeSet::new();
        for binder in binders {
            if binder.is_empty() || !exact_binders.insert(binder.clone()) {
                return Err(SourceValueTransferError::InvalidRelocatableTypeParameter {
                    name: binder.clone(),
                });
            }
        }
        Ok(Self {
            facts,
            registry,
            binders: exact_binders,
            active_nominals: BTreeSet::new(),
            native_depth: 0,
        })
    }

    pub(super) fn plan(
        mut self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Result<ValueTransferPlan, SourceValueTransferError> {
        self.classify(module_path, ty, &BTreeMap::new())
            .map(Classification::into_plan)
    }

    pub(super) fn classify(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        match ty {
            TypeRefIr::Builtin { name, args } => {
                self.classify_builtin(module_path, name, args, substitutions)
            }
            TypeRefIr::LocalType { type_index } => {
                self.classify_local_type(module_path, *type_index, substitutions)
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self.classify_publication_type(module_path, *type_index, substitutions),
            TypeRefIr::ServiceSymbol { symbol } => {
                self.classify_service_type(module_path, symbol, substitutions)
            }
            TypeRefIr::PackageSymbol { symbol } => {
                self.classify_package_type(module_path, symbol, substitutions)
            }
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => Err(package_schema_error(
                package_id,
                stable_schema_key,
                package_schema_type_id,
            )),
            TypeRefIr::AppliedNominal { base, arguments } => {
                self.classify_applied_type(module_path, base, arguments, substitutions)
            }
            TypeRefIr::DbObjectSymbol { symbol } => {
                Err(SourceValueTransferError::DatabaseObjectUnsupported {
                    module_path: symbol.module_path.clone(),
                    symbol: symbol.symbol.clone(),
                })
            }
            TypeRefIr::Record { fields } => {
                self.classify_record(module_path, fields, substitutions)
            }
            TypeRefIr::Union { items } => self.classify_union(module_path, items, substitutions),
            TypeRefIr::Nullable { inner } => {
                self.classify_nullable(module_path, inner, substitutions)
            }
            TypeRefIr::Literal { value } => {
                self.classify_literal(module_path, value, substitutions)
            }
            TypeRefIr::TypeParam { name } => self.classify_type_parameter(name, substitutions),
            TypeRefIr::AnyInterface { interface } => {
                self.classify_any_interface(module_path, interface, substitutions)
            }
            TypeRefIr::Function { .. } => Err(SourceValueTransferError::CallbackTypeUnsupported),
        }
    }

    fn classify_local_type(
        &mut self,
        module_path: &str,
        type_index: u32,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        if module_path.is_empty() {
            return Err(SourceValueTransferError::MissingLocalTypeOwner { type_index });
        }
        self.classify_nominal(
            SourceValueTransferNominalId::Local {
                module_path: module_path.to_string(),
                type_index,
            },
            NominalTypeRefBaseIr::PublicationType {
                module_path: module_path.to_string(),
                type_index,
            },
            &[],
            module_path,
            substitutions,
        )
    }

    fn classify_publication_type(
        &mut self,
        module_path: &str,
        type_index: u32,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        if module_path.is_empty() {
            return Err(SourceValueTransferError::MissingPublicationTypeOwner { type_index });
        }
        self.classify_nominal(
            SourceValueTransferNominalId::Publication {
                module_path: module_path.to_string(),
                type_index,
            },
            NominalTypeRefBaseIr::PublicationType {
                module_path: module_path.to_string(),
                type_index,
            },
            &[],
            module_path,
            substitutions,
        )
    }

    fn classify_service_type(
        &mut self,
        argument_module: &str,
        symbol: &ServiceSymbolRef,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        validate_service_symbol(symbol)?;
        self.classify_nominal(
            SourceValueTransferNominalId::ServiceSymbol {
                module_path: symbol.module_path.clone(),
                symbol: symbol.symbol.clone(),
            },
            NominalTypeRefBaseIr::ServiceSymbol {
                symbol: symbol.clone(),
            },
            &[],
            argument_module,
            substitutions,
        )
    }

    fn classify_package_type(
        &mut self,
        module_path: &str,
        symbol: &PackageSymbolRef,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        let stable_symbol = self.stable_package_symbol(symbol)?;
        if let Some(classification) = self.classify_privileged_package(stable_symbol.clone(), &[]) {
            return classification;
        }
        if self.registry_owns_package_symbol(&stable_symbol) {
            return self.classify_registry_package(module_path, stable_symbol, &[], substitutions);
        }
        self.classify_nominal(
            package_nominal_id(&stable_symbol),
            NominalTypeRefBaseIr::PackageSymbol {
                symbol: stable_symbol,
            },
            &[],
            module_path,
            substitutions,
        )
    }

    fn classify_applied_type(
        &mut self,
        module_path: &str,
        base: &NominalTypeRefBaseIr,
        arguments: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        let stable_base = self.stable_nominal_base(module_path, base)?;
        if let NominalTypeRefBaseIr::PackageSymbol { symbol } = &stable_base {
            if let Some(classification) =
                self.classify_privileged_package(symbol.clone(), arguments)
            {
                return classification;
            }
            if self.registry_owns_package_symbol(symbol) {
                return self.classify_registry_package(
                    module_path,
                    symbol.clone(),
                    arguments,
                    substitutions,
                );
            }
        }
        self.classify_nominal(
            nominal_base_id(base, module_path)?,
            stable_base,
            arguments,
            module_path,
            substitutions,
        )
    }

    fn classify_record(
        &mut self,
        module_path: &str,
        fields: &BTreeMap<String, TypeRefIr>,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        let mut stable_fields = BTreeMap::new();
        let mut deferred = false;
        for (field, ty) in fields {
            let position = SourceValueTransferPosition::AnonymousRecordField {
                field: field.clone(),
            };
            let classified = self.classify_at(module_path, ty, substitutions, position.clone())?;
            deferred |= self.require_ordinary_snapshot(&classified, position)?;
            stable_fields.insert(field.clone(), classified.ty);
        }
        Ok(ordinary_aggregate(
            TypeRefIr::Record {
                fields: stable_fields,
            },
            deferred,
        ))
    }

    fn classify_union(
        &mut self,
        module_path: &str,
        items: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        if items.is_empty() {
            return Err(SourceValueTransferError::EmptyUnion {
                owner: "anonymous union".to_string(),
            });
        }
        let mut stable_items = Vec::with_capacity(items.len());
        let mut deferred = false;
        for (index, item) in items.iter().enumerate() {
            let position = SourceValueTransferPosition::AnonymousUnionItem { index };
            let classified =
                self.classify_at(module_path, item, substitutions, position.clone())?;
            deferred |= self.require_ordinary_snapshot(&classified, position)?;
            stable_items.push(classified.ty);
        }
        Ok(ordinary_aggregate(
            TypeRefIr::Union {
                items: stable_items,
            },
            deferred,
        ))
    }

    fn classify_nullable(
        &mut self,
        module_path: &str,
        inner: &TypeRefIr,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        let position = SourceValueTransferPosition::NullableInner;
        let classified = self.classify_at(module_path, inner, substitutions, position.clone())?;
        let deferred = self.require_ordinary_snapshot(&classified, position)?;
        Ok(ordinary_aggregate(
            TypeRefIr::Nullable {
                inner: Box::new(classified.ty),
            },
            deferred,
        ))
    }

    fn classify_literal(
        &mut self,
        module_path: &str,
        value: &LiteralIr,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        let builtin = match value {
            LiteralIr::Null => "null",
            LiteralIr::Bool { .. } => "bool",
            LiteralIr::Number { .. } => "number",
            LiteralIr::String { .. } => "string",
        };
        let mut classified = self.classify_builtin(module_path, builtin, &[], substitutions)?;
        classified.ty = TypeRefIr::Literal {
            value: value.clone(),
        };
        Ok(classified)
    }

    fn classify_type_parameter(
        &self,
        name: &str,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        if let Some(classified) = substitutions.get(name) {
            return Ok(classified.clone());
        }
        if !self.binders.contains(name) {
            return Err(SourceValueTransferError::UnresolvedTypeParameter {
                name: name.to_string(),
            });
        }
        Ok(Classification::deferred(
            TypeRefIr::TypeParam {
                name: name.to_string(),
            },
            None,
            None,
        ))
    }

    fn classify_any_interface(
        &mut self,
        module_path: &str,
        interface: &skiff_artifact_model::InterfaceInstantiationRef,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        if interface.interface_abi_id.is_empty() {
            return Err(SourceValueTransferError::MissingInterfaceAbiIdentity);
        }
        let mut stable_arguments = Vec::with_capacity(interface.canonical_type_args.len());
        let mut deferred = false;
        for (index, argument) in interface.canonical_type_args.iter().enumerate() {
            let position = SourceValueTransferPosition::AnyInterfaceTypeArgument { index };
            let classified =
                self.classify_at(module_path, argument, substitutions, position.clone())?;
            deferred |= self.require_ordinary_snapshot(&classified, position)?;
            stable_arguments.push(classified.ty);
        }
        Ok(ordinary_aggregate(
            TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: interface.interface_abi_id.clone(),
                    canonical_type_args: stable_arguments,
                },
            },
            deferred,
        ))
    }

    fn classify_nominal(
        &mut self,
        nominal: SourceValueTransferNominalId,
        stable_base: NominalTypeRefBaseIr,
        arguments: &[TypeRefIr],
        argument_module: &str,
        outer_substitutions: &BTreeMap<String, Classification>,
    ) -> Result<Classification, SourceValueTransferError> {
        if matches!(nominal, SourceValueTransferNominalId::PackageSchema { .. }) {
            return Err(SourceValueTransferError::PackageSchemaUnsupported { nominal });
        }
        let fact = self.facts.nominal(&nominal).cloned().ok_or_else(|| {
            SourceValueTransferError::MissingNominalFacts {
                nominal: nominal.clone(),
            }
        })?;
        validate_nominal_parameters(&nominal, &fact.type_parameters)?;
        if fact.type_parameters.len() != arguments.len() {
            return Err(SourceValueTransferError::NominalArityMismatch {
                nominal,
                expected: fact.type_parameters.len(),
                actual: arguments.len(),
            });
        }
        if fact.declaration_module.is_empty() {
            return Err(SourceValueTransferError::MissingNominalDeclarationModule { nominal });
        }
        match &fact.semantics {
            SourceValueTransferNominalSemantics::Actor => {
                return Err(SourceValueTransferError::ActorUnsupported { nominal });
            }
            SourceValueTransferNominalSemantics::NativeOpaque
            | SourceValueTransferNominalSemantics::Capability => {
                return Err(SourceValueTransferError::NativeNominalNotRegistered { nominal });
            }
            SourceValueTransferNominalSemantics::Ordinary(_) => {}
        }

        let mut stable_arguments = Vec::with_capacity(arguments.len());
        let mut substitutions = BTreeMap::new();
        let mut deferred = false;
        for (index, (parameter, argument)) in fact.type_parameters.iter().zip(arguments).enumerate()
        {
            let position = SourceValueTransferPosition::NominalTypeArgument {
                nominal: nominal.clone(),
                index,
            };
            let classified = self.classify_at(
                argument_module,
                argument,
                outer_substitutions,
                position.clone(),
            )?;
            deferred |= self.require_ordinary_snapshot(&classified, position)?;
            stable_arguments.push(classified.ty.clone());
            substitutions.insert(parameter.clone(), classified);
        }

        let stable_ty = nominal_type_ref(stable_base, stable_arguments);
        if !self.active_nominals.insert(nominal.clone()) {
            return Err(SourceValueTransferError::RecursiveNominal { nominal });
        }
        let descriptor_result = self.classify_nominal_descriptor(&nominal, &fact, &substitutions);
        self.active_nominals.remove(&nominal);
        let descriptor = descriptor_result?;
        deferred |= descriptor.deferred;

        if deferred {
            return Ok(Classification::deferred(
                stable_ty,
                Some(ValueTransferPlanKind::SnapshotShare),
                Some(NativeValueEmbedding::Ordinary),
            ));
        }
        let plan = descriptor
            .alias_plan
            .unwrap_or(ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            });
        Ok(Classification::concrete(
            stable_ty,
            plan,
            NativeValueEmbedding::Ordinary,
        ))
    }

    fn classify_nominal_descriptor(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        fact: &SourceValueTransferNominalFact,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<DescriptorProof, SourceValueTransferError> {
        let SourceValueTransferNominalSemantics::Ordinary(descriptor) = &fact.semantics else {
            unreachable!("non-ordinary nominal semantics returned before descriptor proof")
        };
        match descriptor {
            TypeDescriptorIr::Record { fields } => {
                self.prove_nominal_record(nominal, &fact.declaration_module, fields, substitutions)
            }
            TypeDescriptorIr::Representation { representation } => self
                .prove_nominal_representation(
                    nominal,
                    &fact.declaration_module,
                    representation,
                    substitutions,
                ),
            TypeDescriptorIr::Union { branches } => {
                self.prove_nominal_union(nominal, &fact.declaration_module, branches, substitutions)
            }
            TypeDescriptorIr::Alias { target } => {
                self.prove_nominal_alias(nominal, &fact.declaration_module, target, substitutions)
            }
            TypeDescriptorIr::Interface => {
                Err(SourceValueTransferError::InterfaceNominalUnsupported {
                    nominal: nominal.clone(),
                })
            }
        }
    }

    fn prove_nominal_record(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        module_path: &str,
        fields: &BTreeMap<String, TypeRefIr>,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<DescriptorProof, SourceValueTransferError> {
        let mut deferred = false;
        for (field, ty) in fields {
            deferred |= self.prove_ordinary_at(
                module_path,
                ty,
                substitutions,
                SourceValueTransferPosition::NominalRecordField {
                    nominal: nominal.clone(),
                    field: field.clone(),
                },
            )?;
        }
        Ok(DescriptorProof::aggregate(deferred))
    }

    fn prove_nominal_representation(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        module_path: &str,
        representation: &TypeRefIr,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<DescriptorProof, SourceValueTransferError> {
        let deferred = self.prove_ordinary_at(
            module_path,
            representation,
            substitutions,
            SourceValueTransferPosition::NominalRepresentation {
                nominal: nominal.clone(),
            },
        )?;
        Ok(DescriptorProof::aggregate(deferred))
    }

    fn prove_nominal_union(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        module_path: &str,
        branches: &[NamedUnionBranchIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<DescriptorProof, SourceValueTransferError> {
        if branches.is_empty() {
            return Err(SourceValueTransferError::EmptyUnion {
                owner: nominal.to_string(),
            });
        }
        let mut deferred = false;
        for (index, branch) in branches.iter().enumerate() {
            let ty = match branch {
                NamedUnionBranchIr::ConcreteNominal { nominal_type } => nominal_type,
                NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => payload_type,
                NamedUnionBranchIr::Literal { .. } => continue,
            };
            deferred |= self.prove_ordinary_at(
                module_path,
                ty,
                substitutions,
                SourceValueTransferPosition::NominalUnionBranch {
                    nominal: nominal.clone(),
                    index,
                },
            )?;
        }
        Ok(DescriptorProof::aggregate(deferred))
    }

    fn prove_nominal_alias(
        &mut self,
        nominal: &SourceValueTransferNominalId,
        module_path: &str,
        target: &TypeRefIr,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<DescriptorProof, SourceValueTransferError> {
        let position = SourceValueTransferPosition::NominalAliasTarget {
            nominal: nominal.clone(),
        };
        let classified = self.classify_at(module_path, target, substitutions, position.clone())?;
        let deferred = self.require_ordinary_snapshot(&classified, position)?;
        Ok(DescriptorProof {
            deferred,
            alias_plan: classified.concrete_plan().cloned(),
        })
    }

    fn prove_ordinary_at(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, Classification>,
        position: SourceValueTransferPosition,
    ) -> Result<bool, SourceValueTransferError> {
        let classified = self.classify_at(module_path, ty, substitutions, position.clone())?;
        self.require_ordinary_snapshot(&classified, position)
    }

    pub(super) fn classify_at(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, Classification>,
        position: SourceValueTransferPosition,
    ) -> Result<Classification, SourceValueTransferError> {
        self.classify(module_path, ty, substitutions)
            .map_err(|source| SourceValueTransferError::AtStructuralPosition {
                position,
                source: Box::new(source),
            })
    }

    pub(super) fn require_ordinary_snapshot(
        &self,
        classified: &Classification,
        position: SourceValueTransferPosition,
    ) -> Result<bool, SourceValueTransferError> {
        if let Some(found) = classified.known_kind() {
            if found != ValueTransferPlanKind::SnapshotShare {
                return Err(
                    SourceValueTransferError::StructuralPositionNotSnapshotShare {
                        position,
                        found,
                    },
                );
            }
        }
        if let Some(found) = classified.embedding {
            if found != NativeValueEmbedding::Ordinary {
                return Err(SourceValueTransferError::StructuralPositionNotOrdinary {
                    position,
                    found,
                });
            }
        }
        Ok(classified.is_deferred())
    }

    pub(super) fn stable_type_ref(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<TypeRefIr, SourceValueTransferError> {
        match ty {
            TypeRefIr::Builtin { name, args } => {
                self.stable_builtin_type(module_path, name, args, substitutions)
            }
            TypeRefIr::LocalType { type_index } => {
                if module_path.is_empty() {
                    return Err(SourceValueTransferError::MissingLocalTypeOwner {
                        type_index: *type_index,
                    });
                }
                Ok(TypeRefIr::PublicationType {
                    module_path: module_path.to_string(),
                    type_index: *type_index,
                })
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                if module_path.is_empty() {
                    return Err(SourceValueTransferError::MissingPublicationTypeOwner {
                        type_index: *type_index,
                    });
                }
                Ok(ty.clone())
            }
            TypeRefIr::ServiceSymbol { symbol } => {
                validate_service_symbol(symbol)?;
                Ok(ty.clone())
            }
            TypeRefIr::PackageSymbol { symbol } => Ok(TypeRefIr::PackageSymbol {
                symbol: self.stable_package_symbol(symbol)?,
            }),
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => Err(package_schema_error(
                package_id,
                stable_schema_key,
                package_schema_type_id,
            )),
            TypeRefIr::AppliedNominal { base, arguments } => Ok(TypeRefIr::AppliedNominal {
                base: self.stable_nominal_base(module_path, base)?,
                arguments: arguments
                    .iter()
                    .map(|argument| self.stable_type_ref(module_path, argument, substitutions))
                    .collect::<Result<_, _>>()?,
            }),
            TypeRefIr::DbObjectSymbol { symbol } => {
                Err(SourceValueTransferError::DatabaseObjectUnsupported {
                    module_path: symbol.module_path.clone(),
                    symbol: symbol.symbol.clone(),
                })
            }
            TypeRefIr::Record { fields } => {
                self.stable_record_type(module_path, fields, substitutions)
            }
            TypeRefIr::Union { items } => self.stable_union_type(module_path, items, substitutions),
            TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
                inner: Box::new(self.stable_type_ref(module_path, inner, substitutions)?),
            }),
            TypeRefIr::Literal { .. } => Ok(ty.clone()),
            TypeRefIr::TypeParam { name } => {
                if let Some(classified) = substitutions.get(name) {
                    return Ok(classified.ty.clone());
                }
                if !self.binders.contains(name) {
                    return Err(SourceValueTransferError::UnresolvedTypeParameter {
                        name: name.clone(),
                    });
                }
                Ok(ty.clone())
            }
            TypeRefIr::AnyInterface { interface } => {
                self.stable_any_interface(module_path, interface, substitutions)
            }
            TypeRefIr::Function { .. } => Err(SourceValueTransferError::CallbackTypeUnsupported),
        }
    }

    fn stable_builtin_type(
        &self,
        module_path: &str,
        name: &str,
        arguments: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<TypeRefIr, SourceValueTransferError> {
        let Some(builtin) = canonical_file_ir_builtin(name) else {
            return Err(SourceValueTransferError::UnknownBuiltin {
                name: name.to_string(),
            });
        };
        if arguments.len() != builtin.arity {
            return Err(SourceValueTransferError::BuiltinArityMismatch {
                builtin: builtin.canonical_name.to_string(),
                expected: builtin.arity,
                actual: arguments.len(),
            });
        }
        Ok(TypeRefIr::Builtin {
            name: builtin.canonical_name.to_string(),
            args: arguments
                .iter()
                .map(|argument| self.stable_type_ref(module_path, argument, substitutions))
                .collect::<Result<_, _>>()?,
        })
    }

    fn stable_record_type(
        &self,
        module_path: &str,
        fields: &BTreeMap<String, TypeRefIr>,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<TypeRefIr, SourceValueTransferError> {
        Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    Ok((
                        name.clone(),
                        self.stable_type_ref(module_path, ty, substitutions)?,
                    ))
                })
                .collect::<Result<_, SourceValueTransferError>>()?,
        })
    }

    fn stable_union_type(
        &self,
        module_path: &str,
        items: &[TypeRefIr],
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<TypeRefIr, SourceValueTransferError> {
        Ok(TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| self.stable_type_ref(module_path, item, substitutions))
                .collect::<Result<_, _>>()?,
        })
    }

    fn stable_any_interface(
        &self,
        module_path: &str,
        interface: &skiff_artifact_model::InterfaceInstantiationRef,
        substitutions: &BTreeMap<String, Classification>,
    ) -> Result<TypeRefIr, SourceValueTransferError> {
        if interface.interface_abi_id.is_empty() {
            return Err(SourceValueTransferError::MissingInterfaceAbiIdentity);
        }
        Ok(TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|argument| self.stable_type_ref(module_path, argument, substitutions))
                    .collect::<Result<_, _>>()?,
            },
        })
    }

    pub(super) fn stable_nominal_base(
        &self,
        module_path: &str,
        base: &NominalTypeRefBaseIr,
    ) -> Result<NominalTypeRefBaseIr, SourceValueTransferError> {
        match base {
            NominalTypeRefBaseIr::LocalType { type_index } => {
                if module_path.is_empty() {
                    return Err(SourceValueTransferError::MissingLocalTypeOwner {
                        type_index: *type_index,
                    });
                }
                Ok(NominalTypeRefBaseIr::PublicationType {
                    module_path: module_path.to_string(),
                    type_index: *type_index,
                })
            }
            NominalTypeRefBaseIr::PublicationType {
                module_path,
                type_index,
            } => {
                if module_path.is_empty() {
                    return Err(SourceValueTransferError::MissingPublicationTypeOwner {
                        type_index: *type_index,
                    });
                }
                Ok(base.clone())
            }
            NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
                validate_service_symbol(symbol)?;
                Ok(base.clone())
            }
            NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                Ok(NominalTypeRefBaseIr::PackageSymbol {
                    symbol: self.stable_package_symbol(symbol)?,
                })
            }
            NominalTypeRefBaseIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => Err(SourceValueTransferError::PackageSchemaUnsupported {
                nominal: SourceValueTransferNominalId::PackageSchema {
                    package_id: package_id.clone(),
                    stable_schema_key: stable_schema_key.clone(),
                    package_schema_type_id: package_schema_type_id.as_str().to_string(),
                },
            }),
        }
    }
}

struct DescriptorProof {
    deferred: bool,
    alias_plan: Option<ValueTransferPlan>,
}

impl DescriptorProof {
    const fn aggregate(deferred: bool) -> Self {
        Self {
            deferred,
            alias_plan: None,
        }
    }
}

fn ordinary_aggregate(ty: TypeRefIr, deferred: bool) -> Classification {
    if deferred {
        Classification::deferred(
            ty,
            Some(ValueTransferPlanKind::SnapshotShare),
            Some(NativeValueEmbedding::Ordinary),
        )
    } else {
        Classification::concrete(
            ty,
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            },
            NativeValueEmbedding::Ordinary,
        )
    }
}

fn validate_nominal_parameters(
    nominal: &SourceValueTransferNominalId,
    parameters: &[String],
) -> Result<(), SourceValueTransferError> {
    let mut seen = BTreeSet::new();
    for parameter in parameters {
        if parameter.is_empty() || !seen.insert(parameter) {
            return Err(SourceValueTransferError::InvalidNominalTypeParameter {
                nominal: nominal.clone(),
                parameter: parameter.clone(),
            });
        }
    }
    Ok(())
}

fn validate_service_symbol(symbol: &ServiceSymbolRef) -> Result<(), SourceValueTransferError> {
    if symbol.module_path.is_empty() {
        return Err(SourceValueTransferError::InvalidServiceSymbol {
            field: "modulePath",
        });
    }
    if symbol.symbol.is_empty() {
        return Err(SourceValueTransferError::InvalidServiceSymbol { field: "symbol" });
    }
    Ok(())
}

fn package_schema_error(
    package_id: &str,
    stable_schema_key: &str,
    package_schema_type_id: &skiff_artifact_model::PackageSchemaTypeId,
) -> SourceValueTransferError {
    SourceValueTransferError::PackageSchemaUnsupported {
        nominal: SourceValueTransferNominalId::PackageSchema {
            package_id: package_id.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: package_schema_type_id.as_str().to_string(),
        },
    }
}

fn package_nominal_id(symbol: &PackageSymbolRef) -> SourceValueTransferNominalId {
    SourceValueTransferNominalId::PackageSymbol {
        package: package_ref(&symbol.package),
        symbol_path: symbol.symbol_path.clone(),
        abi_expectation: symbol.abi_expectation.clone(),
    }
}

fn package_ref(package: &PackageRefIr) -> SourceValueTransferPackageRef {
    match package {
        PackageRefIr::PackageId { package_id } => {
            SourceValueTransferPackageRef::PackageId(package_id.clone())
        }
        PackageRefIr::Dependency { dependency_ref } => {
            SourceValueTransferPackageRef::Dependency(dependency_ref.clone())
        }
    }
}

fn nominal_base_id(
    base: &NominalTypeRefBaseIr,
    owner_module: &str,
) -> Result<SourceValueTransferNominalId, SourceValueTransferError> {
    match base {
        NominalTypeRefBaseIr::LocalType { type_index } => {
            if owner_module.is_empty() {
                return Err(SourceValueTransferError::MissingLocalTypeOwner {
                    type_index: *type_index,
                });
            }
            Ok(SourceValueTransferNominalId::Local {
                module_path: owner_module.to_string(),
                type_index: *type_index,
            })
        }
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => Ok(SourceValueTransferNominalId::Publication {
            module_path: module_path.clone(),
            type_index: *type_index,
        }),
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            Ok(SourceValueTransferNominalId::ServiceSymbol {
                module_path: symbol.module_path.clone(),
                symbol: symbol.symbol.clone(),
            })
        }
        NominalTypeRefBaseIr::PackageSymbol { symbol } => Ok(package_nominal_id(symbol)),
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(SourceValueTransferNominalId::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.as_str().to_string(),
        }),
    }
}

fn nominal_type_ref(base: NominalTypeRefBaseIr, arguments: Vec<TypeRefIr>) -> TypeRefIr {
    if arguments.is_empty() {
        match base {
            NominalTypeRefBaseIr::LocalType { type_index } => TypeRefIr::LocalType { type_index },
            NominalTypeRefBaseIr::PublicationType {
                module_path,
                type_index,
            } => TypeRefIr::PublicationType {
                module_path,
                type_index,
            },
            NominalTypeRefBaseIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol { symbol },
            NominalTypeRefBaseIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol { symbol },
            NominalTypeRefBaseIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            },
        }
    } else {
        TypeRefIr::AppliedNominal { base, arguments }
    }
}
