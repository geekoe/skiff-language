use std::{cell::RefCell, collections::BTreeMap};

use crate::file_ir::{
    AssignTargetIr, BoxSourceIr, CallTargetIr, DbBodyIr, DbChangeIr, DbLeaseClaimIr, DbLeaseReadIr,
    DbOperationIr, DbPredicateIr, DbQueryIr, DbQueryValueIr, DbSelectorIr, DbTransactionIr, ExprIr,
    FileIrUnit, InterfaceDeclIr, PackageRefIr, PatternIr, StmtIr, TestEffectOutcomeIr,
    TypeDescriptorIr, TypeRefIr,
};
use skiff_artifact_identity::{canonical_interface_method_abi_id, type_ref_abi_key};
use skiff_artifact_model::{
    InterfaceInstantiationRef, NamedUnionBranchIr, NominalTypeRefBaseIr, PackageSymbolRef,
};
use skiff_compiler_source::TypeResolutionModel;

use super::external_refs::rebuild_external_refs_for_file_ir_unit;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationTypeRefLocation {
    module_path: String,
    type_index: u32,
}

#[derive(Debug, Default)]
struct PublicationLocalRefIndex<'a> {
    current_package_id: Option<String>,
    package_dependency_abi_expectations: BTreeMap<String, String>,
    package_dependency_abi_expectations_by_package_id: BTreeMap<String, String>,
    types_by_module_symbol: BTreeMap<(String, String), PublicationTypeRefLocation>,
    type_resolution: Option<&'a TypeResolutionModel>,
    alias_expansion_error: RefCell<Option<String>>,
}

impl<'a> PublicationLocalRefIndex<'a> {
    fn build(
        units: &[FileIrUnit],
        current_package_id: Option<&str>,
        type_resolution: Option<&'a TypeResolutionModel>,
        package_dependency_abi_expectations: &BTreeMap<String, String>,
        package_dependency_abi_expectations_by_package_id: &BTreeMap<String, String>,
    ) -> Self {
        let mut index = Self {
            current_package_id: current_package_id.map(str::to_string),
            package_dependency_abi_expectations: package_dependency_abi_expectations.clone(),
            package_dependency_abi_expectations_by_package_id:
                package_dependency_abi_expectations_by_package_id.clone(),
            type_resolution,
            alias_expansion_error: RefCell::new(None),
            ..Self::default()
        };
        for unit in units {
            for (symbol, declaration) in &unit.declarations.types {
                index.types_by_module_symbol.insert(
                    (unit.module_path.clone(), symbol.clone()),
                    PublicationTypeRefLocation {
                        module_path: unit.module_path.clone(),
                        type_index: declaration.type_index,
                    },
                );
            }
        }
        index
    }

    fn type_location(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<&PublicationTypeRefLocation> {
        self.types_by_module_symbol
            .get(&(module_path.to_string(), symbol.to_string()))
    }

    fn current_package_type_location(
        &self,
        package: &PackageRefIr,
        symbol_path: &str,
    ) -> Option<&PublicationTypeRefLocation> {
        let PackageRefIr::PackageId { package_id } = package else {
            return None;
        };
        if self.current_package_id.as_deref() != Some(package_id.as_str()) {
            return None;
        }
        let symbol_path = symbol_path.strip_prefix("root.").unwrap_or(symbol_path);
        let (module_path, symbol) = symbol_path.rsplit_once('.')?;
        self.type_location(module_path, symbol)
    }
}

pub(super) fn rewrite_publication_local_refs(
    units: &mut [FileIrUnit],
    current_package_id: Option<&str>,
    type_resolution: Option<&TypeResolutionModel>,
    package_dependency_abi_expectations: &BTreeMap<String, String>,
    package_dependency_abi_expectations_by_package_id: &BTreeMap<String, String>,
) -> Result<(), String> {
    let index = PublicationLocalRefIndex::build(
        units,
        current_package_id,
        type_resolution,
        package_dependency_abi_expectations,
        package_dependency_abi_expectations_by_package_id,
    );
    for unit in units {
        let module_path = unit.module_path.clone();
        rewrite_unit(&index, &module_path, unit);
        if let Some(error) = index.alias_expansion_error.borrow_mut().take() {
            return Err(error);
        }
        rebuild_external_refs_for_file_ir_unit(unit).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn rewrite_unit(index: &PublicationLocalRefIndex, module_path: &str, unit: &mut FileIrUnit) {
    for ty in &mut unit.type_table {
        rewrite_type_descriptor(index, module_path, &mut ty.descriptor);
        for implemented in &mut ty.implements {
            rewrite_type_ref(index, module_path, implemented);
        }
    }

    for interface in unit.declarations.interfaces.values_mut() {
        rewrite_interface_decl(index, module_path, interface);
    }
    for declaration in unit.declarations.constants.values_mut() {
        rewrite_type_ref(index, module_path, &mut declaration.ty);
    }
    for declaration in unit.declarations.db.values_mut() {
        rewrite_type_ref(index, module_path, &mut declaration.type_ref);
        rewrite_type_ref(index, module_path, &mut declaration.key.ty);
        for field in &mut declaration.fields {
            rewrite_type_ref(index, module_path, &mut field.ty);
        }
    }

    for constant in &mut unit.constants {
        rewrite_type_ref(index, module_path, &mut constant.ty);
        rewrite_body(index, module_path, &mut constant.body);
    }
    for executable in &mut unit.executables {
        for param in &mut executable.params {
            rewrite_type_ref(index, module_path, &mut param.ty);
        }
        rewrite_type_ref(index, module_path, &mut executable.return_type);
        if let Some(self_type) = &mut executable.self_type {
            rewrite_type_ref(index, module_path, self_type);
        }
        rewrite_body(index, module_path, &mut executable.body);
    }
}

fn rewrite_interface_decl(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    interface: &mut InterfaceDeclIr,
) {
    for operation in &mut interface.operations {
        for param in &mut operation.params {
            rewrite_type_ref(index, module_path, &mut param.ty);
        }
        rewrite_type_ref(index, module_path, &mut operation.return_type);
        if let Some(implicit_self) = &mut operation.implicit_self {
            rewrite_type_ref(index, module_path, implicit_self);
        }
    }
}

fn rewrite_type_descriptor(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    descriptor: &mut TypeDescriptorIr,
) {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for field in fields.values_mut() {
                rewrite_type_ref(index, module_path, field);
            }
        }
        TypeDescriptorIr::Alias { target } => {
            rewrite_type_ref(index, module_path, target);
        }
        TypeDescriptorIr::Representation { representation } => {
            rewrite_type_ref(index, module_path, representation);
        }
        TypeDescriptorIr::Union { branches } => {
            for branch in branches {
                match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        rewrite_type_ref(index, module_path, nominal_type);
                    }
                    NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                        rewrite_type_ref(index, module_path, payload_type);
                    }
                    NamedUnionBranchIr::Literal { .. } => {}
                }
            }
        }
        TypeDescriptorIr::Interface => {}
    }
}

fn rewrite_body(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    body: &mut crate::file_ir::ExecutableBody,
) {
    for stmt in &mut body.statements {
        rewrite_stmt(index, module_path, stmt);
    }
    for expr in &mut body.expressions {
        rewrite_expr(index, module_path, expr);
    }
}

fn rewrite_stmt(index: &PublicationLocalRefIndex, module_path: &str, stmt: &mut StmtIr) {
    match stmt {
        StmtIr::ForIn { item_type, .. } => {
            if let Some(item_type) = item_type {
                rewrite_type_ref(index, module_path, item_type);
            }
        }
        StmtIr::Match { arms, .. } => {
            for arm in arms {
                rewrite_pattern(index, module_path, &mut arm.pattern);
            }
        }
        StmtIr::Throw { payload_type, .. } => {
            rewrite_type_ref(index, module_path, payload_type);
        }
        StmtIr::TestEffectRegister {
            expect,
            step_expect,
            outcome,
            ..
        } => {
            if let Some(expect) = expect {
                rewrite_type_ref(index, module_path, &mut expect.request_type);
            }
            if let Some(step_expect) = step_expect {
                rewrite_type_ref(index, module_path, &mut step_expect.request_type);
            }
            match outcome {
                TestEffectOutcomeIr::Respond { value_type, .. } => {
                    rewrite_type_ref(index, module_path, value_type);
                }
                TestEffectOutcomeIr::Throw { payload_type, .. } => {
                    rewrite_type_ref(index, module_path, payload_type);
                }
                TestEffectOutcomeIr::Stream { item_type, .. } => {
                    rewrite_type_ref(index, module_path, item_type);
                }
            }
        }
        StmtIr::Assign {
            target: AssignTargetIr::ActorSelfField { field_type, .. },
            ..
        } => {
            rewrite_type_ref(index, module_path, field_type);
        }
        StmtIr::Let { .. }
        | StmtIr::Assign { .. }
        | StmtIr::Timeout { .. }
        | StmtIr::Concurrent { .. }
        | StmtIr::If { .. }
        | StmtIr::While { .. }
        | StmtIr::Assert { .. }
        | StmtIr::Break
        | StmtIr::Continue
        | StmtIr::Dispatch { .. }
        | StmtIr::Emit { .. }
        | StmtIr::Expr { .. }
        | StmtIr::Return { .. }
        | StmtIr::Rethrow { .. } => {}
    }
}

fn rewrite_pattern(index: &PublicationLocalRefIndex, module_path: &str, pattern: &mut PatternIr) {
    match pattern {
        PatternIr::Type { ty } => {
            rewrite_type_ref(index, module_path, ty);
        }
        PatternIr::Record { fields } => {
            for field in fields {
                rewrite_pattern(index, module_path, &mut field.pattern);
            }
        }
        PatternIr::Wildcard | PatternIr::Literal { .. } | PatternIr::Binding { .. } => {}
    }
}

fn rewrite_expr(index: &PublicationLocalRefIndex, module_path: &str, expr: &mut ExprIr) {
    match expr {
        ExprIr::Construct { type_ref, .. } | ExprIr::RepresentationWrap { type_ref, .. } => {
            rewrite_type_ref(index, module_path, type_ref);
        }
        ExprIr::InterfaceBox {
            interface, source, ..
        } => {
            rewrite_interface_instantiation_ref(index, module_path, interface);
            rewrite_box_source(index, module_path, source);
        }
        ExprIr::Call { call } => {
            rewrite_call_target(index, module_path, &mut call.target);
            if !is_actor_registry_native_call(call) {
                for ty in call.type_args.values_mut() {
                    rewrite_type_ref(index, module_path, ty);
                }
            }
        }
        ExprIr::Throw { payload_type, .. } => {
            rewrite_type_ref(index, module_path, payload_type);
        }
        ExprIr::Catch { catch_type, .. } => {
            rewrite_type_ref(index, module_path, catch_type);
        }
        ExprIr::DbOperation { operation } => {
            rewrite_db_operation(index, module_path, operation);
        }
        ExprIr::DbQuery { query } => {
            rewrite_db_query_value(index, module_path, query);
        }
        ExprIr::DbTransaction { transaction } => {
            rewrite_db_transaction(index, module_path, transaction);
        }
        ExprIr::DbLeaseClaim { claim } => {
            rewrite_db_lease_claim(index, module_path, claim);
        }
        ExprIr::DbLeaseRead { read } => {
            rewrite_db_lease_read(index, module_path, read);
        }
        ExprIr::ActorSelfField { field_type, .. } => {
            rewrite_type_ref(index, module_path, field_type);
        }
        ExprIr::Literal { .. }
        | ExprIr::LoadSlot { .. }
        | ExprIr::LoadConst { .. }
        | ExprIr::LoadPackageConst { .. }
        | ExprIr::Field { .. }
        | ExprIr::MapLiteral { .. }
        | ExprIr::ArrayLiteral { .. }
        | ExprIr::Unary { .. }
        | ExprIr::Binary { .. }
        | ExprIr::Rethrow { .. }
        | ExprIr::Timeout { .. }
        | ExprIr::ValueBlock { .. }
        | ExprIr::ConcurrentValue { .. } => {}
    }
}

fn is_actor_registry_native_call(call: &skiff_artifact_model::CallIr) -> bool {
    match &call.target {
        skiff_artifact_model::CallTargetIr::Native { target } => {
            target.binding_key.as_deref() == Some("std.actor.get")
        }
        skiff_artifact_model::CallTargetIr::PackageCallable {
            package_callable_id,
            ..
        } => package_callable_id.as_str().ends_with(":std.actor.get"),
        _ => false,
    }
}

fn rewrite_call_target(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    target: &mut CallTargetIr,
) {
    match target {
        CallTargetIr::InterfaceMethod {
            interface,
            method_abi_id,
            ..
        } => {
            let changed = rewrite_interface_instantiation_ref(index, module_path, interface);
            if changed {
                if let Some((_, method_name)) = method_abi_id.rsplit_once(':') {
                    *method_abi_id = canonical_interface_method_abi_id(interface, method_name);
                }
            }
        }
        CallTargetIr::LocalExecutable { .. }
        | CallTargetIr::PublicationExecutable { .. }
        | CallTargetIr::ServiceDependencySymbol { .. }
        | CallTargetIr::ServiceCall { .. }
        | CallTargetIr::PackageCallable { .. }
        | CallTargetIr::ActorMethod { .. }
        | CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. } => {}
    }
}

fn rewrite_box_source(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    source: &mut BoxSourceIr,
) {
    match source {
        BoxSourceIr::Local {
            concrete_type,
            method_table,
        } => {
            rewrite_type_ref(index, module_path, concrete_type);
            let changed = rewrite_interface_instantiation_ref(
                index,
                module_path,
                &mut method_table.interface,
            );
            rewrite_type_ref(index, module_path, &mut method_table.concrete_type);
            for slot in &mut method_table.slots {
                for param in &mut slot.signature.params {
                    rewrite_type_ref(index, module_path, &mut param.ty);
                }
                rewrite_type_ref(index, module_path, &mut slot.signature.return_type);
                if changed {
                    slot.method_abi_id = canonical_interface_method_abi_id(
                        &method_table.interface,
                        &slot.method_name,
                    );
                }
            }
        }
        BoxSourceIr::Remote { operations, .. } => {
            let changed =
                rewrite_interface_instantiation_ref(index, module_path, &mut operations.interface);
            for slot in &mut operations.slots {
                for param in &mut slot.signature.params {
                    rewrite_type_ref(index, module_path, &mut param.ty);
                }
                rewrite_type_ref(index, module_path, &mut slot.signature.return_type);
                if changed {
                    if let Some((_, method_name)) = slot.method_abi_id.rsplit_once(':') {
                        slot.method_abi_id =
                            canonical_interface_method_abi_id(&operations.interface, method_name);
                    }
                }
            }
        }
    }
}

fn rewrite_db_operation(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    operation: &mut DbOperationIr,
) {
    rewrite_type_ref(index, module_path, &mut operation.target.type_ref);
    rewrite_type_ref(index, module_path, &mut operation.result_type);
    if let Some(selector) = &mut operation.selector {
        rewrite_db_selector(index, module_path, selector);
    }
    if let Some(query) = &mut operation.query {
        rewrite_db_query(index, module_path, query);
    }
    if let Some(body) = &mut operation.body {
        rewrite_db_body(index, module_path, body);
    }
    if let Some(body) = &mut operation.insert_body {
        rewrite_db_body(index, module_path, body);
    }
    if let Some(change) = &mut operation.change {
        rewrite_db_change(index, module_path, change);
    }
}

fn rewrite_db_query_value(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    query: &mut DbQueryValueIr,
) {
    rewrite_type_ref(index, module_path, &mut query.target.type_ref);
    rewrite_type_ref(index, module_path, &mut query.result_type);
    rewrite_db_query(index, module_path, &mut query.query);
}

fn rewrite_db_transaction(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    transaction: &mut DbTransactionIr,
) {
    rewrite_type_ref(index, module_path, &mut transaction.result_type);
}

fn rewrite_db_lease_claim(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    claim: &mut DbLeaseClaimIr,
) {
    rewrite_type_ref(index, module_path, &mut claim.target.type_ref);
    rewrite_type_ref(index, module_path, &mut claim.result_type);
}

fn rewrite_db_lease_read(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    read: &mut DbLeaseReadIr,
) {
    rewrite_type_ref(index, module_path, &mut read.target.type_ref);
    rewrite_type_ref(index, module_path, &mut read.result_type);
}

fn rewrite_db_selector(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    selector: &mut DbSelectorIr,
) {
    match selector {
        DbSelectorIr::Query { query } => rewrite_db_query(index, module_path, query),
        DbSelectorIr::Key { .. } => {}
    }
}

fn rewrite_db_query(index: &PublicationLocalRefIndex, module_path: &str, query: &mut DbQueryIr) {
    for predicate in &mut query.where_clauses {
        rewrite_db_predicate(index, module_path, predicate);
    }
}

fn rewrite_db_predicate(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    predicate: &mut DbPredicateIr,
) {
    match predicate {
        DbPredicateIr::And { predicates } | DbPredicateIr::Or { predicates } => {
            for predicate in predicates {
                rewrite_db_predicate(index, module_path, predicate);
            }
        }
        DbPredicateIr::Not { predicate } | DbPredicateIr::Conditional { predicate, .. } => {
            rewrite_db_predicate(index, module_path, predicate);
        }
        DbPredicateIr::Compare { .. } | DbPredicateIr::Regex { .. } => {}
    }
}

fn rewrite_db_body(_index: &PublicationLocalRefIndex, _module_path: &str, _body: &mut DbBodyIr) {}

fn rewrite_db_change(
    _index: &PublicationLocalRefIndex,
    _module_path: &str,
    _change: &mut DbChangeIr,
) {
}

fn rewrite_interface_instantiation_ref(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    interface: &mut InterfaceInstantiationRef,
) -> bool {
    let mut changed = false;
    if let Ok(mut interface_identity) =
        serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
    {
        let mut identity_changed = rewrite_type_ref(index, module_path, &mut interface_identity);
        if let TypeRefIr::LocalType { type_index } = &interface_identity {
            interface_identity = TypeRefIr::PublicationType {
                module_path: module_path.to_string(),
                type_index: *type_index,
            };
            identity_changed = true;
        }
        if identity_changed {
            interface.interface_abi_id = type_ref_abi_key(&interface_identity);
            changed = true;
        }
    }
    for arg in &mut interface.canonical_type_args {
        changed |= rewrite_type_ref(index, module_path, arg);
    }
    changed
}

fn rewrite_type_ref(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    ty: &mut TypeRefIr,
) -> bool {
    let mut changed = false;
    if let Some(type_resolution) = index.type_resolution {
        match type_resolution.expand_alias_type_ref_for_module(module_path, ty) {
            Ok(expanded) => {
                if expanded != *ty {
                    *ty = expanded;
                    changed = true;
                }
            }
            Err(error) => {
                let mut expansion_error = index.alias_expansion_error.borrow_mut();
                if expansion_error.is_none() {
                    *expansion_error = Some(format!(
                        "alias expansion failed in module {module_path}: {error}"
                    ));
                }
                return false;
            }
        }
    }

    let nested_changed = match ty {
        TypeRefIr::ServiceSymbol { symbol } => {
            if let Some(location) = index.type_location(&symbol.module_path, &symbol.symbol) {
                rewrite_type_ref_to_publication_location(module_path, ty, location)
            } else {
                false
            }
        }
        TypeRefIr::PackageSymbol { symbol } => {
            if let Some(location) =
                index.current_package_type_location(&symbol.package, &symbol.symbol_path)
            {
                rewrite_type_ref_to_publication_location(module_path, ty, location)
            } else {
                rewrite_external_package_symbol(index, symbol)
            }
        }
        TypeRefIr::Builtin { args, .. } => {
            let mut changed = false;
            for arg in args {
                changed |= rewrite_type_ref(index, module_path, arg);
            }
            changed
        }
        TypeRefIr::AppliedNominal { base, arguments } => {
            let mut changed = rewrite_applied_nominal_base(index, module_path, base);
            for argument in arguments {
                changed |= rewrite_type_ref(index, module_path, argument);
            }
            changed
        }
        TypeRefIr::Record { fields } => {
            let mut changed = false;
            for field in fields.values_mut() {
                changed |= rewrite_type_ref(index, module_path, field);
            }
            changed
        }
        TypeRefIr::Union { items } => {
            let mut changed = false;
            for item in items {
                changed |= rewrite_type_ref(index, module_path, item);
            }
            changed
        }
        TypeRefIr::Nullable { inner } => rewrite_type_ref(index, module_path, inner),
        TypeRefIr::AnyInterface { interface } => {
            rewrite_interface_instantiation_ref(index, module_path, interface)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            let mut changed = false;
            for param in params {
                changed |= rewrite_type_ref(index, module_path, &mut param.ty);
            }
            changed |= rewrite_type_ref(index, module_path, return_type);
            changed
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    };
    changed || nested_changed
}

fn rewrite_applied_nominal_base(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    base: &mut NominalTypeRefBaseIr,
) -> bool {
    match base {
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            let Some(location) = index.type_location(&symbol.module_path, &symbol.symbol) else {
                return false;
            };
            *base = if location.module_path == module_path {
                NominalTypeRefBaseIr::LocalType {
                    type_index: location.type_index,
                }
            } else {
                NominalTypeRefBaseIr::PublicationType {
                    module_path: location.module_path.clone(),
                    type_index: location.type_index,
                }
            };
            true
        }
        NominalTypeRefBaseIr::PackageSymbol { symbol } => {
            if let Some(location) =
                index.current_package_type_location(&symbol.package, &symbol.symbol_path)
            {
                *base = if location.module_path == module_path {
                    NominalTypeRefBaseIr::LocalType {
                        type_index: location.type_index,
                    }
                } else {
                    NominalTypeRefBaseIr::PublicationType {
                        module_path: location.module_path.clone(),
                        type_index: location.type_index,
                    }
                };
                true
            } else {
                rewrite_external_package_symbol(index, symbol)
            }
        }
        NominalTypeRefBaseIr::LocalType { .. }
        | NominalTypeRefBaseIr::PublicationType { .. }
        | NominalTypeRefBaseIr::PackageSchema { .. } => false,
    }
}

fn rewrite_external_package_symbol(
    index: &PublicationLocalRefIndex<'_>,
    symbol: &mut PackageSymbolRef,
) -> bool {
    let mut changed = false;
    let abi_expectation = match &mut symbol.package {
        PackageRefIr::Dependency { dependency_ref } => {
            changed = canonicalize_dependency_ref(index, dependency_ref);
            let Some(abi_expectation) = index
                .package_dependency_abi_expectations
                .get(dependency_ref)
            else {
                return changed;
            };
            abi_expectation
        }
        PackageRefIr::PackageId { package_id } => {
            let Some(abi_expectation) = index
                .package_dependency_abi_expectations_by_package_id
                .get(package_id)
            else {
                return false;
            };
            abi_expectation
        }
    };
    if symbol.abi_expectation.as_ref() == Some(abi_expectation) {
        changed
    } else {
        symbol.abi_expectation = Some(abi_expectation.clone());
        true
    }
}

fn canonicalize_dependency_ref(
    index: &PublicationLocalRefIndex<'_>,
    dependency_ref: &mut String,
) -> bool {
    let Some(type_resolution) = index.type_resolution else {
        return false;
    };
    let canonical = type_resolution.canonical_package_dependency_ref(dependency_ref);
    if canonical == dependency_ref {
        return false;
    }
    *dependency_ref = canonical.to_string();
    true
}

fn rewrite_type_ref_to_publication_location(
    module_path: &str,
    ty: &mut TypeRefIr,
    location: &PublicationTypeRefLocation,
) -> bool {
    if location.module_path == module_path {
        *ty = TypeRefIr::LocalType {
            type_index: location.type_index,
        };
    } else {
        *ty = TypeRefIr::PublicationType {
            module_path: location.module_path.clone(),
            type_index: location.type_index,
        };
    }
    true
}

#[cfg(test)]
mod tests;
