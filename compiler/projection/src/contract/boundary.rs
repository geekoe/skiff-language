use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ExecutableIr, InterfaceInstantiationRef, InterfaceOperationIr, LiteralIr, ParamIr, TypeDeclIr,
    TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::type_ref::{
    walk_type_ref_with_path, TypeRefVisit, TypeRefVisitPath, TypeRefVisitPathSegment,
};
use skiff_compiler_projection_input::{ProjectionSourceSymbolKey, ProjectionView};

use super::{ContractOperationBindingProjection, ContractProjection, ContractProjectionIndex};

mod type_closure;

use type_closure::recursive_type_violations;
pub(crate) use type_closure::BoundaryTypeRefClosureValidator;

// Service, DB/persistent, spawn, queue, and runtime-wire checks share the
// TypeRef policy traversal below. BoundaryKind names the crossing; policy
// differences should stay keyed by that enum so recoverable boundary planning
// can build on one static gate.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractBoundaryError {
    violations: Vec<String>,
}

impl ContractBoundaryError {
    pub fn message(&self) -> String {
        self.violations
            .iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn validate_contract_projection_boundary(
    input: ProjectionView<'_>,
    projection: &ContractProjection,
) -> Result<(), ContractBoundaryError> {
    let mut validator = ContractBoundaryValidator {
        index: ContractProjectionIndex::from_projection_input_with_prelude(
            input,
            Some(projection.prelude()),
        ),
        projection,
        violations: Vec::new(),
    };
    validator.validate();
    validator.finish()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryTypePolicyDecision {
    Accept,
    Reject(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryKind {
    PackageLinkEntry,
    PackageSchema,
    ServiceOperation,
    ServicePublicInstanceOperation,
    RuntimeWireSchema,
    PublicApiPayload,
    Materialization,
    RecoverablePayload,
    DbPayload,
    PersistentSchema,
    SpawnPayload,
    QueueWorkItemPayload,
    QueuePayload,
    Internal,
}

impl BoundaryKind {
    pub fn allows_any_interface(self) -> bool {
        matches!(
            self,
            Self::PackageLinkEntry
                | Self::PackageSchema
                | Self::DbPayload
                | Self::RecoverablePayload
                | Self::SpawnPayload
                | Self::QueueWorkItemPayload
                | Self::QueuePayload
                | Self::Internal
        )
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::PackageLinkEntry => "package link entry boundary",
            Self::PackageSchema => "package public schema boundary",
            Self::ServiceOperation => "service operation boundary schema",
            Self::ServicePublicInstanceOperation => {
                "service public instance operation boundary schema"
            }
            Self::RuntimeWireSchema => "runtime wire boundary schema",
            Self::PublicApiPayload => "public API payload boundary",
            Self::Materialization => "materialization boundary",
            Self::RecoverablePayload => "recoverable payload boundary",
            Self::DbPayload => "DB payload boundary",
            Self::PersistentSchema => "persistent payload schema",
            Self::SpawnPayload => "spawn payload boundary",
            Self::QueueWorkItemPayload => "queue work item payload boundary",
            Self::QueuePayload => "queue payload boundary",
            Self::Internal => "internal boundary",
        }
    }
}

pub fn validate_type_ref_boundary_policy(
    ty: &TypeRefIr,
    boundary_kind: BoundaryKind,
    policy: &mut impl FnMut(BoundaryKind, TypeRefVisit<'_>) -> BoundaryTypePolicyDecision,
) -> Result<(), String> {
    let mut rejection = None;
    walk_type_ref_with_path(ty, &mut |visit| {
        if rejection.is_some() {
            return;
        }
        match policy(boundary_kind, visit) {
            BoundaryTypePolicyDecision::Accept => {}
            BoundaryTypePolicyDecision::Reject(message) => rejection = Some(message),
        }
    });
    rejection.map_or(Ok(()), Err)
}

pub(crate) fn validate_static_type_ref_boundary_policy(
    ty: &TypeRefIr,
    boundary_kind: BoundaryKind,
) -> Result<(), String> {
    validate_type_ref_boundary_policy(ty, boundary_kind, &mut |kind, visit| {
        static_type_ref_boundary_policy(kind, visit)
    })
}

pub(crate) fn static_type_ref_boundary_policy(
    boundary_kind: BoundaryKind,
    visit: TypeRefVisit<'_>,
) -> BoundaryTypePolicyDecision {
    let path = display_type_ref_visit_path(&visit.path);
    match visit.ty {
        TypeRefIr::AnyInterface { .. } if !boundary_kind.allows_any_interface() => {
            BoundaryTypePolicyDecision::Reject(format!(
                "any interface value type {} cannot be used in {} at {}",
                display_policy_type_ref(visit.ty),
                boundary_kind.description(),
                path
            ))
        }
        TypeRefIr::Function { .. } => BoundaryTypePolicyDecision::Reject(format!(
            "callback function type {} cannot be used in {} at {}",
            display_policy_type_ref(visit.ty),
            boundary_kind.description(),
            path
        )),
        TypeRefIr::Native { name, .. } if name == "Stream" => {
            BoundaryTypePolicyDecision::Reject(format!(
                "Stream<T> cannot be used in {} at {}",
                boundary_kind.description(),
                path
            ))
        }
        TypeRefIr::Native { name, .. } => {
            if let Some(request_local_name) = request_local_boundary_type_name(name) {
                BoundaryTypePolicyDecision::Reject(format!(
                    "request-local type {request_local_name} cannot be used in {} at {}",
                    boundary_kind.description(),
                    path
                ))
            } else {
                BoundaryTypePolicyDecision::Accept
            }
        }
        _ => BoundaryTypePolicyDecision::Accept,
    }
}
fn contract_index_type_decl_for_type_ref<'a>(
    index: &'a ContractProjectionIndex<'a>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<(&'a str, &'a TypeDeclIr)> {
    match ty {
        TypeRefIr::LocalType { type_index } => {
            let unit = index.unit_by_module_path(module_path)?;
            unit.type_table
                .get(*type_index as usize)
                .map(|decl| (unit.module_path.as_str(), decl))
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => {
            let unit = index.unit_by_module_path(module_path)?;
            unit.type_table
                .get(*type_index as usize)
                .map(|decl| (unit.module_path.as_str(), decl))
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            if let Some(source_key) =
                index.source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
            {
                if let Some(binding) = index.type_binding_by_source_key(&source_key) {
                    return Some((binding.module_path, binding.type_decl));
                }
            }
            let source_module = index.source_module_for_reference_module(&symbol.module_path);
            let unit = index.unit_by_module_path(source_module)?;
            let declaration = unit.declarations.types.get(&symbol.symbol)?;
            unit.type_table
                .get(declaration.type_index as usize)
                .map(|decl| (unit.module_path.as_str(), decl))
        }
        TypeRefIr::Native { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => None,
    }
}

struct ContractBoundaryValidator<'a> {
    index: ContractProjectionIndex<'a>,
    projection: &'a ContractProjection,
    violations: Vec<String>,
}

impl ContractBoundaryValidator<'_> {
    fn validate(&mut self) {
        self.validate_projected_types();
        self.validate_projected_aliases();
        self.validate_projected_interfaces();
        self.collect_recursive_type_violations();
    }

    fn finish(mut self) -> Result<(), ContractBoundaryError> {
        if self.violations.is_empty() {
            return Ok(());
        }
        self.violations.sort();
        self.violations.dedup();
        Err(ContractBoundaryError {
            violations: self.violations,
        })
    }

    fn validate_projected_types(&mut self) {
        for ty in self.projection.types.values() {
            let Some(decl) =
                self.type_decl_by_module_local_name(&ty.source_module, &ty.source_name)
            else {
                continue;
            };
            let descriptor = decl.descriptor.clone();
            let discriminator = decl.discriminator.clone();
            match descriptor {
                TypeDescriptorIr::Record { fields } => {
                    for field_ty in fields.values() {
                        self.validate_nested_boundary_type_ref(
                            &ty.source_module,
                            field_ty,
                            BoundaryKind::RuntimeWireSchema,
                        );
                    }
                }
                TypeDescriptorIr::Union { variants } => {
                    self.validate_named_union_boundary_type_refs(
                        &ty.public_name,
                        &ty.source_module,
                        &variants,
                        discriminator.as_deref(),
                    );
                }
                TypeDescriptorIr::Alias { target } => {
                    self.validate_named_alias_boundary_type_ref(
                        &ty.public_name,
                        &ty.source_module,
                        &target,
                        discriminator.as_deref(),
                    );
                }
                TypeDescriptorIr::Native { .. } => {}
            }
        }
    }

    fn validate_projected_aliases(&mut self) {
        for alias in self.projection.aliases.values() {
            let Some(decl) =
                self.type_decl_by_module_local_name(&alias.source_module, &alias.source_name)
            else {
                continue;
            };
            let TypeDescriptorIr::Alias { target } = &decl.descriptor else {
                continue;
            };
            let target = target.clone();
            let discriminator = decl.discriminator.clone();
            self.validate_named_alias_boundary_type_ref(
                &alias.public_name,
                &alias.source_module,
                &target,
                discriminator.as_deref(),
            );
        }
    }

    fn validate_projected_interfaces(&mut self) {
        for interface in self.projection.interfaces.values() {
            if let Some(interface_decl) = self.index.interface_decl_by_module_local_name(
                &interface.source_module,
                &interface.source_name,
            ) {
                for operation in &interface_decl.operations {
                    self.validate_interface_operation(
                        &interface.source_module,
                        operation,
                        BoundaryKind::ServiceOperation,
                    );
                }
                continue;
            }

            let Some(api_binding) = self.projection.api_bindings.get(&interface.public_name) else {
                continue;
            };
            for operation_binding in api_binding.operations.values() {
                self.validate_synthetic_operation(
                    operation_binding,
                    BoundaryKind::ServicePublicInstanceOperation,
                );
            }
        }
    }

    fn validate_interface_operation(
        &mut self,
        module_path: &str,
        operation: &InterfaceOperationIr,
        boundary_kind: BoundaryKind,
    ) {
        for param in &operation.params {
            self.validate_nested_boundary_type_ref(module_path, &param.ty, boundary_kind);
        }
        self.validate_operation_return_boundary_type_ref(
            module_path,
            &operation.return_type,
            boundary_kind,
        );
    }

    fn validate_synthetic_operation(
        &mut self,
        binding: &ContractOperationBindingProjection,
        boundary_kind: BoundaryKind,
    ) {
        let Some(executable) = self
            .index
            .executable_by_module_symbol(&binding.module_path, &binding.executable_symbol)
        else {
            return;
        };
        for param in self.executable_params_without_receiver(binding, executable) {
            self.validate_nested_boundary_type_ref(&binding.module_path, &param.ty, boundary_kind);
        }
        self.validate_operation_return_boundary_type_ref(
            &binding.module_path,
            &executable.return_type,
            boundary_kind,
        );
    }

    fn executable_params_without_receiver<'a>(
        &self,
        binding: &ContractOperationBindingProjection,
        executable: &'a ExecutableIr,
    ) -> &'a [ParamIr] {
        if executable.params.first().is_some_and(|param| {
            param.name == "self"
                && self.receiver_type_matches(
                    &binding.module_path,
                    &param.ty,
                    &binding.module_path,
                    &binding.type_name,
                )
        }) {
            &executable.params[1..]
        } else {
            &executable.params
        }
    }

    fn receiver_type_matches(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        expected_module: &str,
        expected_type: &str,
    ) -> bool {
        match ty {
            TypeRefIr::LocalType { type_index } => self
                .type_decl_by_module_index(module_path, *type_index)
                .is_some_and(|decl| module_path == expected_module && decl.name == expected_type),
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self
                .type_decl_by_module_index(module_path, *type_index)
                .is_some_and(|decl| module_path == expected_module && decl.name == expected_type),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                self.source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
                    .is_some_and(|source_key| {
                        source_key.module_path() == expected_module
                            && source_key.symbol() == expected_type
                    })
                    || {
                        let source_module =
                            self.source_module_for_reference_module(&symbol.module_path);
                        source_module == expected_module && symbol.symbol == expected_type
                    }
            }
            TypeRefIr::Native { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::Function { .. } => false,
        }
    }

    fn validate_nested_boundary_type_ref(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        boundary_kind: BoundaryKind,
    ) {
        self.validate_boundary_type_ref(module_path, ty, false, None, boundary_kind);
    }

    fn validate_operation_return_boundary_type_ref(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        boundary_kind: BoundaryKind,
    ) {
        self.validate_boundary_type_ref(module_path, ty, true, None, boundary_kind);
    }

    fn validate_named_alias_boundary_type_ref(
        &mut self,
        public_name: &str,
        module_path: &str,
        ty: &TypeRefIr,
        discriminator: Option<&str>,
    ) {
        if discriminator.is_some() && !self.type_ref_is_union(ty) {
            self.violations.push(format!(
                "type {public_name} discriminator can only be used with a named union representation"
            ));
        }
        if let Some(variants) = self.type_ref_union_items(ty) {
            self.validate_named_union_shape(public_name, module_path, variants, discriminator);
        }
        self.validate_boundary_type_ref(
            module_path,
            ty,
            false,
            discriminator,
            BoundaryKind::RuntimeWireSchema,
        );
        if self.type_ref_is_anonymous_record(module_path, ty) {
            self.violations.push(format!(
                "anonymous record type cannot be used as boundary representation {public_name}; use a named record or a named discriminator union"
            ));
        }
    }

    fn validate_named_union_boundary_type_refs(
        &mut self,
        public_name: &str,
        module_path: &str,
        variants: &[TypeRefIr],
        discriminator: Option<&str>,
    ) {
        self.validate_named_union_shape(public_name, module_path, variants, discriminator);
        for variant in variants {
            if let Some(discriminator) = discriminator {
                if self.type_ref_is_anonymous_record(module_path, variant) {
                    self.validate_anonymous_discriminator_record_branch(
                        module_path,
                        variant,
                        discriminator,
                    );
                    continue;
                }
            }
            self.validate_nested_boundary_type_ref(
                module_path,
                variant,
                BoundaryKind::RuntimeWireSchema,
            );
        }
        if let Some(discriminator) = discriminator {
            self.validate_anonymous_discriminator_union_branch_values(
                public_name,
                module_path,
                variants,
                discriminator,
            );
        }
    }

    fn validate_named_union_shape(
        &mut self,
        public_name: &str,
        module_path: &str,
        variants: &[TypeRefIr],
        discriminator: Option<&str>,
    ) {
        let has_anonymous_record_branch = variants
            .iter()
            .any(|variant| self.type_ref_is_anonymous_record(module_path, variant));
        if discriminator.is_some() && variants.len() <= 1 {
            self.violations.push(format!(
                "type {public_name} discriminator can only be used with a named union representation"
            ));
        }
        if discriminator.is_some() && variants.len() > 1 && !has_anonymous_record_branch {
            self.violations.push(format!(
                "type {public_name} discriminator can only be used with anonymous record union branches"
            ));
        }
        if discriminator.is_none() && variants.len() > 1 && has_anonymous_record_branch {
            self.violations.push(format!(
                "named union type {public_name} uses anonymous record branches; add discriminator \"tag\" to the type declaration"
            ));
        }
    }

    fn validate_boundary_type_ref(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        allow_outer_stream: bool,
        anonymous_record_discriminator: Option<&str>,
        boundary_kind: BoundaryKind,
    ) {
        if let Err(message) =
            self.validate_any_interface_boundary_policy(module_path, ty, boundary_kind)
        {
            self.violations.push(message);
            return;
        }
        match ty {
            TypeRefIr::Native { name, args } => {
                self.validate_builtin_boundary_type_ref(
                    module_path,
                    name,
                    args,
                    allow_outer_stream,
                    boundary_kind,
                );
            }
            TypeRefIr::LocalType { type_index } => {
                let Some(decl) = self.type_decl_by_module_index(module_path, *type_index) else {
                    self.violations.push(format!(
                        "unknown type {} cannot be used in service boundary schema",
                        self.display_type_ref(module_path, ty)
                    ));
                    return;
                };
                let _ = decl;
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let Some(decl) = self.type_decl_by_module_index(module_path, *type_index) else {
                    self.violations.push(format!(
                        "unknown type {} cannot be used in service boundary schema",
                        self.display_type_ref(module_path, ty)
                    ));
                    return;
                };
                let _ = decl;
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                if self
                    .source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
                    .and_then(|source_key| {
                        self.type_decl_by_module_local_name(
                            source_key.module_path(),
                            source_key.symbol(),
                        )
                    })
                    .is_some()
                {
                    return;
                }
                let source_module = self.source_module_for_reference_module(&symbol.module_path);
                if self
                    .type_decl_by_module_local_name(source_module, &symbol.symbol)
                    .is_some()
                {
                    return;
                }
                self.validate_boundary_standard_library_type_root(&symbol.symbol);
            }
            TypeRefIr::PackageSymbol { .. } => {}
            TypeRefIr::Record { fields } => {
                if let Some(discriminator) = anonymous_record_discriminator {
                    self.validate_anonymous_discriminator_fields(
                        module_path,
                        fields,
                        discriminator,
                    );
                } else {
                    self.reject_anonymous_record(fields);
                }
            }
            TypeRefIr::Union { items } => {
                for item in items {
                    if self.type_ref_is_null(item) {
                        continue;
                    }
                    if anonymous_record_discriminator.is_some()
                        && self.type_ref_is_anonymous_record(module_path, item)
                    {
                        self.validate_boundary_type_ref(
                            module_path,
                            item,
                            false,
                            anonymous_record_discriminator,
                            boundary_kind,
                        );
                    } else {
                        self.validate_nested_boundary_type_ref(module_path, item, boundary_kind);
                    }
                }
            }
            TypeRefIr::Nullable { inner } => {
                self.validate_nested_boundary_type_ref(module_path, inner, boundary_kind);
            }
            TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => {}
            TypeRefIr::AnyInterface { interface } => {
                if boundary_kind.allows_any_interface() {
                    for arg in &interface.canonical_type_args {
                        self.validate_nested_boundary_type_ref(module_path, arg, boundary_kind);
                    }
                } else {
                    self.violations.push(format!(
                        "any interface value types cannot be used in {}",
                        boundary_kind.description()
                    ));
                }
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                self.violations.push(
                    "callback function types cannot be used in service boundary schema".to_string(),
                );
                for param in params {
                    self.validate_nested_boundary_type_ref(module_path, &param.ty, boundary_kind);
                }
                self.validate_nested_boundary_type_ref(module_path, return_type, boundary_kind);
            }
        }
    }

    fn validate_any_interface_boundary_policy(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        boundary_kind: BoundaryKind,
    ) -> Result<(), String> {
        validate_type_ref_boundary_policy(ty, boundary_kind, &mut |kind, visit| match visit.ty {
            TypeRefIr::AnyInterface { .. } if !kind.allows_any_interface() => {
                BoundaryTypePolicyDecision::Reject(format!(
                    "any interface value type {} cannot be used in {} at {}",
                    self.display_type_ref(module_path, visit.ty),
                    kind.description(),
                    display_type_ref_visit_path(&visit.path)
                ))
            }
            _ => BoundaryTypePolicyDecision::Accept,
        })
    }

    fn validate_builtin_boundary_type_ref(
        &mut self,
        module_path: &str,
        name: &str,
        args: &[TypeRefIr],
        allow_outer_stream: bool,
        boundary_kind: BoundaryKind,
    ) {
        if let Some(message) = request_local_boundary_type_violation(name) {
            self.violations.push(message.to_string());
            return;
        }
        if name == "Stream" {
            if !allow_outer_stream {
                self.violations.push(
                    "Stream<T> can only be used as the outer service operation return type"
                        .to_string(),
                );
                return;
            }
            match args {
                [inner] => {
                    self.validate_nested_boundary_type_ref(module_path, inner, boundary_kind)
                }
                _ => self
                    .violations
                    .push("Stream return type must have exactly one type argument".to_string()),
            }
            return;
        }
        if name == "Map" {
            match args {
                [key, value] => {
                    self.collect_request_local_boundary_type_violations(module_path, key);
                    if !self.is_schema_map_key_type(module_path, key) {
                        self.violations.push(format!(
                            "Map key type {} cannot be used in service boundary schema; use string or a representation over string",
                            self.display_type_ref(module_path, key)
                        ));
                    }
                    self.validate_nested_boundary_type_ref(module_path, value, boundary_kind);
                }
                _ => self
                    .violations
                    .push("Map type must have exactly two type arguments".to_string()),
            }
            return;
        }
        self.validate_boundary_standard_library_type_root(name);
        for arg in args {
            self.validate_nested_boundary_type_ref(module_path, arg, boundary_kind);
        }
    }

    fn collect_request_local_boundary_type_violations(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
    ) {
        match ty {
            TypeRefIr::Native { name, args } => {
                if let Some(message) = request_local_boundary_type_violation(name) {
                    self.violations.push(message.to_string());
                }
                for arg in args {
                    self.collect_request_local_boundary_type_violations(module_path, arg);
                }
            }
            TypeRefIr::Record { fields } => {
                for field_type in fields.values() {
                    self.collect_request_local_boundary_type_violations(module_path, field_type);
                }
            }
            TypeRefIr::Union { items } => {
                for item in items {
                    self.collect_request_local_boundary_type_violations(module_path, item);
                }
            }
            TypeRefIr::Nullable { inner } => {
                self.collect_request_local_boundary_type_violations(module_path, inner);
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.collect_request_local_boundary_type_violations(module_path, &param.ty);
                }
                self.collect_request_local_boundary_type_violations(module_path, return_type);
            }
            TypeRefIr::AnyInterface { interface } => {
                for arg in &interface.canonical_type_args {
                    self.collect_request_local_boundary_type_violations(module_path, arg);
                }
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => {}
        }
    }

    fn is_schema_map_key_type(&self, module_path: &str, ty: &TypeRefIr) -> bool {
        if self.type_ref_is_builtin_string(ty) {
            return true;
        }
        if matches!(
            ty,
            TypeRefIr::Nullable { .. } | TypeRefIr::Union { .. } | TypeRefIr::Literal { .. }
        ) {
            return false;
        }
        self.is_nominal_string_map_key_type(module_path, ty, &mut BTreeSet::new())
    }

    fn is_nominal_string_map_key_type(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<ProjectionSourceSymbolKey>,
    ) -> bool {
        let Some((decl_module, decl)) = self.type_decl_for_type_ref(module_path, ty) else {
            return false;
        };
        let key = ProjectionSourceSymbolKey::new(decl_module, &decl.name);
        if !seen.insert(key.clone()) {
            return false;
        }
        let result = match &decl.descriptor {
            TypeDescriptorIr::Alias { target } => {
                self.is_map_key_representation_rhs(decl_module, target, seen)
            }
            TypeDescriptorIr::Union { variants } => variants.iter().all(|variant| {
                self.type_ref_is_builtin_string(variant) || self.type_ref_is_string_literal(variant)
            }),
            TypeDescriptorIr::Record { .. } | TypeDescriptorIr::Native { .. } => false,
        };
        seen.remove(&key);
        result
    }

    fn is_map_key_representation_rhs(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        seen: &mut BTreeSet<ProjectionSourceSymbolKey>,
    ) -> bool {
        if self.type_ref_is_builtin_string(ty) || self.type_ref_is_string_literal(ty) {
            return true;
        }
        match ty {
            TypeRefIr::Nullable { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Native { .. }
            | TypeRefIr::Function { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. } => false,
            TypeRefIr::Union { items } => items.iter().all(|item| {
                self.type_ref_is_builtin_string(item) || self.type_ref_is_string_literal(item)
            }),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. } => {
                self.is_nominal_string_map_key_type(module_path, ty, seen)
            }
            TypeRefIr::Literal { .. } => false,
            TypeRefIr::TypeParam { .. } => false,
        }
    }

    fn validate_anonymous_discriminator_record_branch(
        &mut self,
        module_path: &str,
        ty: &TypeRefIr,
        discriminator: &str,
    ) {
        let TypeRefIr::Record { fields } = ty else {
            return;
        };
        self.validate_anonymous_discriminator_fields(module_path, fields, discriminator);
    }

    fn validate_anonymous_discriminator_fields(
        &mut self,
        module_path: &str,
        fields: &BTreeMap<String, TypeRefIr>,
        discriminator: &str,
    ) {
        let has_discriminator = fields
            .get(discriminator)
            .is_some_and(|field_type| self.type_ref_is_string_literal(field_type));
        if !has_discriminator {
            self.violations.push(format!(
                "anonymous discriminator record union branch {} must declare {discriminator} as a string literal",
                self.display_record_type(fields)
            ));
        }
        for (field_name, field_type) in fields {
            if field_name == discriminator && self.type_ref_is_string_literal(field_type) {
                continue;
            }
            self.validate_nested_boundary_type_ref(
                module_path,
                field_type,
                BoundaryKind::RuntimeWireSchema,
            );
        }
    }

    fn validate_anonymous_discriminator_union_branch_values(
        &mut self,
        public_name: &str,
        _module_path: &str,
        variants: &[TypeRefIr],
        discriminator: &str,
    ) {
        let mut values = BTreeSet::new();
        for variant in variants {
            let TypeRefIr::Record { fields } = variant else {
                continue;
            };
            let Some(value) = fields
                .get(discriminator)
                .and_then(|field_type| self.string_literal_value(field_type))
            else {
                continue;
            };
            if !values.insert(value.clone()) {
                self.violations.push(format!(
                    "anonymous record union branch {discriminator} \"{value}\" in {public_name} must be unique"
                ));
            }
        }
    }

    fn reject_anonymous_record(&mut self, fields: &BTreeMap<String, TypeRefIr>) {
        let field_names = fields.keys().cloned().collect::<Vec<_>>().join(", ");
        self.violations.push(format!(
            "anonymous record type {{{field_names}}} cannot be used in service boundary schema; use a named exported record type"
        ));
    }

    fn validate_boundary_standard_library_type_root(&mut self, root: &str) {
        if is_boundary_primitive(root)
            || is_compiler_owned_boundary_type(root)
            || matches!(root, "Json" | "JsonObject")
        {
            return;
        }
        let prelude = self.projection.prelude();
        if prelude.is_bare_raw_http_envelope_type(root) {
            return;
        }
        if prelude.is_prelude_type_name(root) {
            if !prelude.is_schema_stable_type(root) {
                self.violations.push(format!(
                    "standard_library type {root} is not schema-stable and cannot be used in service boundary schema"
                ));
            }
            return;
        }
        if root.contains('.') {
            if prelude.is_prelude_type_name(root) {
                return;
            }
            if is_compiler_owned_boundary_type(root) {
                return;
            }
        }
        self.violations.push(format!(
            "unknown type {root} cannot be used in service boundary schema"
        ));
    }
    fn collect_recursive_type_violations(&mut self) {
        self.violations
            .extend(recursive_type_violations(&self.index, self.projection));
    }

    fn type_decl_for_type_ref<'a>(
        &'a self,
        module_path: &'a str,
        ty: &'a TypeRefIr,
    ) -> Option<(&'a str, &'a TypeDeclIr)> {
        contract_index_type_decl_for_type_ref(&self.index, module_path, ty)
    }

    fn type_decl_by_module_local_name(
        &self,
        module_path: &str,
        local_name: &str,
    ) -> Option<&TypeDeclIr> {
        self.index
            .type_decl_by_module_local_name(module_path, local_name)
    }

    fn type_decl_by_module_index(&self, module_path: &str, type_index: u32) -> Option<&TypeDeclIr> {
        self.index
            .unit_by_module_path(module_path)?
            .type_table
            .get(type_index as usize)
    }

    fn source_module_for_reference_module<'a>(&'a self, module_path: &'a str) -> &'a str {
        self.index.source_module_for_reference_module(module_path)
    }

    fn source_key_for_reference_symbol(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<ProjectionSourceSymbolKey> {
        self.index
            .source_key_for_reference_symbol(module_path, symbol)
    }

    fn type_ref_is_union(&self, ty: &TypeRefIr) -> bool {
        matches!(ty, TypeRefIr::Union { items } if items.len() > 1)
    }

    fn type_ref_union_items<'a>(&self, ty: &'a TypeRefIr) -> Option<&'a [TypeRefIr]> {
        match ty {
            TypeRefIr::Union { items } if items.len() > 1 => Some(items),
            TypeRefIr::Native { .. }
            | TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::Function { .. } => None,
        }
    }

    fn type_ref_is_anonymous_record(&self, _module_path: &str, ty: &TypeRefIr) -> bool {
        matches!(ty, TypeRefIr::Record { .. })
    }

    fn type_ref_is_builtin_string(&self, ty: &TypeRefIr) -> bool {
        matches!(ty, TypeRefIr::Native { name, args } if args.is_empty() && name == "string")
    }

    fn type_ref_is_string_literal(&self, ty: &TypeRefIr) -> bool {
        matches!(
            ty,
            TypeRefIr::Literal {
                value: LiteralIr::String { .. }
            }
        )
    }

    fn type_ref_is_null(&self, ty: &TypeRefIr) -> bool {
        matches!(
            ty,
            TypeRefIr::Native { name, args } if args.is_empty() && name == "null"
        ) || matches!(
            ty,
            TypeRefIr::Literal {
                value: LiteralIr::Null
            }
        )
    }

    fn string_literal_value(&self, ty: &TypeRefIr) -> Option<String> {
        match ty {
            TypeRefIr::Literal {
                value: LiteralIr::String { value },
            } => Some(value.clone()),
            TypeRefIr::Native { .. }
            | TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::Function { .. } => None,
        }
    }

    fn display_type_ref(&self, module_path: &str, ty: &TypeRefIr) -> String {
        match ty {
            TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
            TypeRefIr::Native { name, args } => format!(
                "{name}<{}>",
                args.iter()
                    .map(|arg| self.display_type_ref(module_path, arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeRefIr::LocalType { type_index } => self
                .type_decl_by_module_index(module_path, *type_index)
                .map(|decl| {
                    self.public_symbol_for_source_key(&ProjectionSourceSymbolKey::new(
                        module_path,
                        &decl.name,
                    ))
                    .unwrap_or_else(|| format!("{module_path}.{}", decl.name))
                })
                .unwrap_or_else(|| format!("<missing:{module_path}:{type_index}>")),
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self
                .type_decl_by_module_index(module_path, *type_index)
                .map(|decl| {
                    self.public_symbol_for_source_key(&ProjectionSourceSymbolKey::new(
                        module_path,
                        &decl.name,
                    ))
                    .unwrap_or_else(|| format!("{module_path}.{}", decl.name))
                })
                .unwrap_or_else(|| format!("<missing publication type:{module_path}>")),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let source_key = self
                    .source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
                    .unwrap_or_else(|| {
                        ProjectionSourceSymbolKey::new(
                            self.source_module_for_reference_module(&symbol.module_path),
                            &symbol.symbol,
                        )
                    });
                let source_module = source_key.module_path();
                let source_symbol = if source_module.is_empty() {
                    source_key.symbol().to_string()
                } else {
                    crate::projection_source_symbol_text(&source_key)
                };
                if source_module.is_empty() {
                    source_symbol
                } else {
                    self.public_symbol_for_source_key(&ProjectionSourceSymbolKey::new(
                        source_module,
                        &symbol.symbol,
                    ))
                    .unwrap_or(source_symbol)
                }
            }
            TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
            TypeRefIr::Record { fields } => self.display_record_type(fields),
            TypeRefIr::Union { items } => items
                .iter()
                .map(|item| self.display_type_ref(module_path, item))
                .collect::<Vec<_>>()
                .join(" | "),
            TypeRefIr::Nullable { inner } => {
                format!("{}?", self.display_type_ref(module_path, inner))
            }
            TypeRefIr::Literal { value } => self.display_literal(value),
            TypeRefIr::TypeParam { name } => name.clone(),
            TypeRefIr::AnyInterface { interface } => {
                self.display_any_interface_type_ref(module_path, interface)
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                let params = params
                    .iter()
                    .map(|param| {
                        format!(
                            "{}: {}",
                            param.name,
                            self.display_type_ref(module_path, &param.ty)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "fn({params}) -> {}",
                    self.display_type_ref(module_path, return_type)
                )
            }
        }
    }

    fn display_record_type(&self, fields: &BTreeMap<String, TypeRefIr>) -> String {
        let fields = fields
            .iter()
            .map(|(name, ty)| format!("{name}: {}", self.display_type_ref("", ty)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{{{fields}}}")
    }

    fn display_literal(&self, value: &LiteralIr) -> String {
        match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => format!("\"{value}\""),
        }
    }

    fn display_any_interface_type_ref(
        &self,
        module_path: &str,
        interface: &InterfaceInstantiationRef,
    ) -> String {
        let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
            .map_or_else(
                |_| interface.interface_abi_id.clone(),
                |ty| self.display_type_ref(module_path, &ty),
            );
        if interface.canonical_type_args.is_empty() {
            format!("any {interface_name}")
        } else {
            format!(
                "any {interface_name}<{}>",
                interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| self.display_type_ref(module_path, arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    fn public_symbol_for_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
    ) -> Option<String> {
        self.index.public_symbol_for_source_key(source_key)
    }
}

fn display_type_ref_visit_path(path: &TypeRefVisitPath) -> String {
    if path.segments().is_empty() {
        return "root".to_string();
    }
    let mut parts = Vec::new();
    for segment in path.segments() {
        parts.push(match segment {
            TypeRefVisitPathSegment::NativeArg { name, index } => {
                format!("{name} type argument {index}")
            }
            TypeRefVisitPathSegment::RecordField { name } => format!("field {name}"),
            TypeRefVisitPathSegment::UnionItem { index } => format!("union item {index}"),
            TypeRefVisitPathSegment::NullableInner => "nullable inner".to_string(),
            TypeRefVisitPathSegment::AnyInterfaceTypeArg { index } => {
                format!("any interface type argument {index}")
            }
            TypeRefVisitPathSegment::FunctionParam { name, index } => {
                format!("function param {name}#{index}")
            }
            TypeRefVisitPathSegment::FunctionReturn => "function return".to_string(),
        });
    }
    parts.join(" -> ")
}

fn display_policy_type_ref(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Native { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(display_policy_type_ref)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::LocalType { type_index } => format!("<local type {type_index}>"),
        TypeRefIr::PublicationType { module_path, .. } => {
            format!("<publication type {module_path}>")
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path()
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::Record { fields } => {
            let fields = fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", display_policy_type_ref(ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{fields}}}")
        }
        TypeRefIr::Union { items } => items
            .iter()
            .map(display_policy_type_ref)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Nullable { inner } => format!("{}?", display_policy_type_ref(inner)),
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => format!("\"{value}\""),
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::AnyInterface { interface } => {
            let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map_or_else(
                    |_| interface.interface_abi_id.clone(),
                    |ty| display_policy_type_ref(&ty),
                );
            if interface.canonical_type_args.is_empty() {
                format!("any {interface_name}")
            } else {
                format!(
                    "any {interface_name}<{}>",
                    interface
                        .canonical_type_args
                        .iter()
                        .map(display_policy_type_ref)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            let params = params
                .iter()
                .map(|param| format!("{}: {}", param.name, display_policy_type_ref(&param.ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn({params}) -> {}", display_policy_type_ref(return_type))
        }
    }
}

fn type_ref_is_null(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Native { name, args } if args.is_empty() && name == "null"
    ) || matches!(
        ty,
        TypeRefIr::Literal {
            value: LiteralIr::Null
        }
    )
}

fn request_local_boundary_type_name(root: &str) -> Option<&'static str> {
    match root {
        "Exception" => Some("Exception<...>"),
        "CatchResult" => Some("CatchResult<...>"),
        _ => None,
    }
}

fn request_local_boundary_type_violation(root: &str) -> Option<&'static str> {
    match root {
        "Exception" => {
            Some("request-local type Exception<...> cannot be used in service boundary schema")
        }
        "CatchResult" => {
            Some("request-local type CatchResult<...> cannot be used in service boundary schema")
        }
        _ => None,
    }
}

fn is_boundary_primitive(ty: &str) -> bool {
    matches!(
        ty,
        "string" | "integer" | "number" | "bool" | "boolean" | "null"
    )
}

fn is_compiler_owned_boundary_type(ty: &str) -> bool {
    matches!(
        ty,
        "ConnectionMessage"
            | "TextConnectionMessage"
            | "BinaryConnectionMessage"
            | "std.websocket.ConnectionMessage"
            | "std.websocket.TextConnectionMessage"
            | "std.websocket.BinaryConnectionMessage"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use skiff_artifact_model::FunctionTypeParamIr;

    fn function_param(name: &str, ty: TypeRefIr) -> FunctionTypeParamIr {
        FunctionTypeParamIr {
            name: name.to_string(),
            ty,
        }
    }

    fn local_type(type_index: u32) -> TypeRefIr {
        TypeRefIr::LocalType { type_index }
    }

    fn type_param(name: &str) -> TypeRefIr {
        TypeRefIr::TypeParam {
            name: name.to_string(),
        }
    }

    fn native_with_args(name: &str, args: Vec<TypeRefIr>) -> TypeRefIr {
        TypeRefIr::Native {
            name: name.to_string(),
            args,
        }
    }

    fn any_interface(args: Vec<TypeRefIr>) -> TypeRefIr {
        TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: "iface".to_string(),
                canonical_type_args: args,
            },
        }
    }

    fn reject_any_interface_boundary_policy(ty: &TypeRefIr) -> String {
        validate_type_ref_boundary_policy(
            ty,
            BoundaryKind::ServiceOperation,
            &mut |_boundary_kind, visit| match visit.ty {
                TypeRefIr::AnyInterface { .. } => BoundaryTypePolicyDecision::Reject(format!(
                    "reject at {}",
                    display_type_ref_visit_path(&visit.path)
                )),
                _ => BoundaryTypePolicyDecision::Accept,
            },
        )
        .unwrap_err()
    }

    #[test]
    fn boundary_policy_wrapper_uses_core_path_traversal() {
        let ty = TypeRefIr::Record {
            fields: BTreeMap::from([(
                "handler".to_string(),
                TypeRefIr::Function {
                    params: vec![function_param("input", type_param("P"))],
                    return_type: Box::new(type_param("R")),
                },
            )]),
        };
        let mut paths = Vec::new();

        validate_type_ref_boundary_policy(
            &ty,
            BoundaryKind::RuntimeWireSchema,
            &mut |kind, visit| {
                assert_eq!(kind, BoundaryKind::RuntimeWireSchema);
                if let TypeRefIr::TypeParam { name } = visit.ty {
                    paths.push((name.clone(), visit.path));
                }
                BoundaryTypePolicyDecision::Accept
            },
        )
        .unwrap();

        assert_eq!(
            paths,
            vec![
                (
                    "P".to_string(),
                    TypeRefVisitPath::empty()
                        .child(TypeRefVisitPathSegment::RecordField {
                            name: "handler".to_string(),
                        })
                        .child(TypeRefVisitPathSegment::FunctionParam {
                            name: "input".to_string(),
                            index: 0,
                        }),
                ),
                (
                    "R".to_string(),
                    TypeRefVisitPath::empty()
                        .child(TypeRefVisitPathSegment::RecordField {
                            name: "handler".to_string(),
                        })
                        .child(TypeRefVisitPathSegment::FunctionReturn),
                ),
            ]
        );
    }

    #[test]
    fn boundary_policy_rejects_any_interface_in_function_signature() {
        let ty = TypeRefIr::Function {
            params: vec![function_param("input", any_interface(Vec::new()))],
            return_type: Box::new(any_interface(Vec::new())),
        };

        let error = reject_any_interface_boundary_policy(&ty);

        assert_eq!(error, "reject at function param input#0");
    }

    #[test]
    fn boundary_policy_rejects_any_interface_root_record_field_and_function_return() {
        assert_eq!(
            reject_any_interface_boundary_policy(&any_interface(Vec::new())),
            "reject at root"
        );

        let record = TypeRefIr::Record {
            fields: BTreeMap::from([("provider".to_string(), any_interface(Vec::new()))]),
        };
        assert_eq!(
            reject_any_interface_boundary_policy(&record),
            "reject at field provider"
        );

        let function_return = TypeRefIr::Function {
            params: vec![function_param("input", TypeRefIr::native("string"))],
            return_type: Box::new(any_interface(Vec::new())),
        };
        assert_eq!(
            reject_any_interface_boundary_policy(&function_return),
            "reject at function return"
        );
    }

    #[test]
    fn owner_internal_recoverable_payload_boundaries_allow_any_interface_values() {
        for boundary_kind in [
            BoundaryKind::DbPayload,
            BoundaryKind::SpawnPayload,
            BoundaryKind::QueueWorkItemPayload,
            BoundaryKind::QueuePayload,
            BoundaryKind::RecoverablePayload,
        ] {
            assert!(boundary_kind.allows_any_interface());
            assert!(validate_static_type_ref_boundary_policy(
                &any_interface(Vec::new()),
                boundary_kind
            )
            .is_ok());
        }
    }

    #[test]
    fn boundary_policy_rejects_any_interface_inside_record_function_closure_with_path() {
        let ty = TypeRefIr::Record {
            fields: BTreeMap::from([(
                "handler".to_string(),
                TypeRefIr::Function {
                    params: vec![function_param("input", any_interface(Vec::new()))],
                    return_type: Box::new(any_interface(Vec::new())),
                },
            )]),
        };

        let error = reject_any_interface_boundary_policy(&ty);

        assert_eq!(error, "reject at field handler -> function param input#0");
    }
}
