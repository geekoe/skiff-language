use std::collections::BTreeMap;

use crate::file_ir::{
    BoxSourceIr, CallTargetIr, DbBodyIr, DbChangeIr, DbLeaseClaimIr, DbLeaseReadIr, DbOperationIr,
    DbPredicateIr, DbQueryIr, DbQueryValueIr, DbSelectorIr, DbTransactionIr, ExprIr,
    ExternalRefTable, FileIrUnit, InterfaceDeclIr, PatternIr, StmtIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_artifact_model::{
    canonical_interface_method_abi_id, type_ref_abi_key, InterfaceInstantiationRef,
};

use super::external_refs::external_refs_for_file_ir_unit;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationTypeRefLocation {
    module_path: String,
    type_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationExecutableRefLocation {
    module_path: String,
    executable_index: u32,
}

#[derive(Debug, Default)]
struct PublicationLocalRefIndex {
    types_by_module_symbol: BTreeMap<(String, String), PublicationTypeRefLocation>,
    executables_by_module_symbol: BTreeMap<(String, String), PublicationExecutableRefLocation>,
}

impl PublicationLocalRefIndex {
    fn build(units: &[FileIrUnit]) -> Self {
        let mut index = Self::default();
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
            for (symbol, declaration) in &unit.declarations.executables {
                index.executables_by_module_symbol.insert(
                    (unit.module_path.clone(), symbol.clone()),
                    PublicationExecutableRefLocation {
                        module_path: unit.module_path.clone(),
                        executable_index: declaration.executable_index,
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

    fn executable_location(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<&PublicationExecutableRefLocation> {
        self.executables_by_module_symbol
            .get(&(module_path.to_string(), symbol.to_string()))
    }
}

pub(super) fn rewrite_publication_local_refs(units: &mut [FileIrUnit]) {
    let index = PublicationLocalRefIndex::build(units);
    for unit in units {
        let module_path = unit.module_path.clone();
        rewrite_unit(&index, &module_path, unit);
        unit.external_refs = ExternalRefTable::default();
        unit.external_refs = external_refs_for_file_ir_unit(unit);
    }
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
        TypeDescriptorIr::Union { variants } => {
            for variant in variants {
                rewrite_type_ref(index, module_path, variant);
            }
        }
        TypeDescriptorIr::Native { .. } => {}
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
        StmtIr::Let { .. }
        | StmtIr::Assign { .. }
        | StmtIr::If { .. }
        | StmtIr::Assert { .. }
        | StmtIr::Break
        | StmtIr::Continue
        | StmtIr::Spawn { .. }
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
        PatternIr::Wildcard | PatternIr::Literal { .. } | PatternIr::Binding { .. } => {}
    }
}

fn rewrite_expr(index: &PublicationLocalRefIndex, module_path: &str, expr: &mut ExprIr) {
    match expr {
        ExprIr::Construct { type_ref, .. } => {
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
            for ty in call.type_args.values_mut() {
                rewrite_type_ref(index, module_path, ty);
            }
        }
        ExprIr::Throw { payload_type, .. } => {
            rewrite_type_ref(index, module_path, payload_type);
        }
        ExprIr::Catch { catch_type, .. } => {
            if let Some(catch_type) = catch_type {
                rewrite_type_ref(index, module_path, catch_type);
            }
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
        ExprIr::Literal { .. }
        | ExprIr::LoadSlot { .. }
        | ExprIr::LoadConst { .. }
        | ExprIr::Field { .. }
        | ExprIr::MapLiteral { .. }
        | ExprIr::ArrayLiteral { .. }
        | ExprIr::Unary { .. }
        | ExprIr::Binary { .. }
        | ExprIr::Rethrow { .. }
        | ExprIr::ValueBlock { .. } => {}
    }
}

fn rewrite_call_target(
    index: &PublicationLocalRefIndex,
    module_path: &str,
    target: &mut CallTargetIr,
) {
    match target {
        CallTargetIr::ExternalServiceSymbol { symbol } => {
            if let Some(location) = index.executable_location(&symbol.module_path, &symbol.symbol) {
                if location.module_path == module_path {
                    *target = CallTargetIr::LocalExecutable {
                        executable_index: location.executable_index,
                    };
                } else {
                    *target = CallTargetIr::PublicationExecutable {
                        module_path: location.module_path.clone(),
                        executable_index: location.executable_index,
                    };
                }
            }
        }
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
        | CallTargetIr::PackageSymbol { .. }
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
        if rewrite_type_ref(index, module_path, &mut interface_identity) {
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
    match ty {
        TypeRefIr::ServiceSymbol { symbol } => {
            if let Some(location) = index.type_location(&symbol.module_path, &symbol.symbol) {
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
            } else {
                false
            }
        }
        TypeRefIr::Native { args, .. } => {
            let mut changed = false;
            for arg in args {
                changed |= rewrite_type_ref(index, module_path, arg);
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
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}
