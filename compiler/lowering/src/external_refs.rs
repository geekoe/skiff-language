use crate::file_ir::{
    AssignTargetIr, BoxSourceIr, CallIr, CallTargetIr, ExecutableBody, ExprIr, ExternalRefTable,
    FileIrUnit, InterfaceDeclIr, MetadataValue, PackageOperationSymbolRef, PatternIr, StmtIr,
    TypeDescriptorIr, TypeRefIr,
};
use skiff_artifact_model::RECEIVER_BUILTIN_CAPABILITY_VERSION;

pub(super) fn required_receiver_builtin_capability_version(unit: &FileIrUnit) -> u32 {
    let has_receiver_builtin = unit
        .constants
        .iter()
        .any(|constant| body_uses_receiver_builtin(&constant.body))
        || unit
            .executables
            .iter()
            .any(|executable| body_uses_receiver_builtin(&executable.body));
    if has_receiver_builtin {
        RECEIVER_BUILTIN_CAPABILITY_VERSION
    } else {
        0
    }
}

fn body_uses_receiver_builtin(body: &ExecutableBody) -> bool {
    body.expressions.iter().any(expr_uses_receiver_builtin)
}

fn expr_uses_receiver_builtin(expr: &ExprIr) -> bool {
    matches!(
        expr,
        ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ReceiverBuiltin { .. },
                ..
            }
        }
    )
}

pub(super) fn external_refs_for_file_ir_unit(unit: &FileIrUnit) -> ExternalRefTable {
    let mut refs = ExternalRefTable::default();
    for ty in &unit.type_table {
        collect_type_ref_external_refs_from_descriptor(&ty.descriptor, &mut refs);
    }
    for interface in unit.declarations.interfaces.values() {
        collect_interface_external_refs(interface, &mut refs);
    }
    for constant in &unit.constants {
        collect_type_ref_external_refs(&constant.ty, &mut refs);
        collect_body_external_refs(&constant.body, &unit.external_refs, &mut refs);
    }
    for executable in &unit.executables {
        for param in &executable.params {
            collect_type_ref_external_refs(&param.ty, &mut refs);
        }
        collect_type_ref_external_refs(&executable.return_type, &mut refs);
        if let Some(self_type) = &executable.self_type {
            collect_type_ref_external_refs(self_type, &mut refs);
        }
        collect_body_external_refs(&executable.body, &unit.external_refs, &mut refs);
    }
    refs
}

fn collect_interface_external_refs(interface: &InterfaceDeclIr, refs: &mut ExternalRefTable) {
    for operation in &interface.operations {
        for param in &operation.params {
            collect_type_ref_external_refs(&param.ty, refs);
        }
        collect_type_ref_external_refs(&operation.return_type, refs);
        if let Some(implicit_self) = &operation.implicit_self {
            collect_type_ref_external_refs(implicit_self, refs);
        }
    }
}

fn collect_body_external_refs(
    body: &ExecutableBody,
    previous_refs: &ExternalRefTable,
    refs: &mut ExternalRefTable,
) {
    for expr in &body.expressions {
        collect_expr_external_refs(expr, previous_refs, refs);
    }
    for stmt in &body.statements {
        collect_stmt_external_refs(stmt, refs);
    }
}

fn collect_stmt_external_refs(stmt: &StmtIr, refs: &mut ExternalRefTable) {
    match stmt {
        StmtIr::Match { arms, .. } => {
            for arm in arms {
                collect_pattern_external_refs(&arm.pattern, refs);
            }
        }
        StmtIr::Assign { target, .. } => collect_assign_target_external_refs(target, refs),
        StmtIr::Let { .. }
        | StmtIr::If { .. }
        | StmtIr::ForIn { .. }
        | StmtIr::Assert { .. }
        | StmtIr::Break
        | StmtIr::Continue
        | StmtIr::Spawn { .. }
        | StmtIr::Emit { .. }
        | StmtIr::Expr { .. }
        | StmtIr::Return { .. }
        | StmtIr::Throw { .. }
        | StmtIr::Rethrow { .. } => {}
    }
}

fn collect_assign_target_external_refs(target: &AssignTargetIr, refs: &mut ExternalRefTable) {
    match target {
        AssignTargetIr::Slot { .. }
        | AssignTargetIr::Field { .. }
        | AssignTargetIr::Index { .. } => {
            let _ = refs;
        }
    }
}

fn collect_expr_external_refs(
    expr: &ExprIr,
    previous_refs: &ExternalRefTable,
    refs: &mut ExternalRefTable,
) {
    match expr {
        ExprIr::Construct { type_ref, .. } => collect_type_ref_external_refs(type_ref, refs),
        ExprIr::InterfaceBox {
            interface, source, ..
        } => {
            for arg in &interface.canonical_type_args {
                collect_type_ref_external_refs(arg, refs);
            }
            collect_box_source_external_refs(source, refs);
        }
        ExprIr::Call { call } => {
            collect_call_target_external_refs(&call.target, refs);
            for ty in call.type_args.values() {
                collect_type_ref_external_refs(ty, refs);
            }
            for metadata in call.metadata.values() {
                collect_metadata_external_refs(metadata, refs);
            }
        }
        ExprIr::Catch { catch_type, .. } => {
            if let Some(ty) = catch_type {
                collect_type_ref_external_refs(ty, refs);
            }
        }
        ExprIr::DbOperation { operation } => {
            collect_type_ref_external_refs(&operation.target.type_ref, refs);
            collect_type_ref_external_refs(&operation.result_type, refs);
        }
        ExprIr::DbQuery { query } => {
            collect_type_ref_external_refs(&query.target.type_ref, refs);
            collect_type_ref_external_refs(&query.result_type, refs);
        }
        ExprIr::DbTransaction { transaction } => {
            collect_type_ref_external_refs(&transaction.result_type, refs);
        }
        ExprIr::DbLeaseClaim { claim } => {
            collect_type_ref_external_refs(&claim.target.type_ref, refs);
            collect_type_ref_external_refs(&claim.result_type, refs);
        }
        ExprIr::DbLeaseRead { read } => {
            collect_type_ref_external_refs(&read.target.type_ref, refs);
            collect_type_ref_external_refs(&read.result_type, refs);
        }
        ExprIr::Literal { .. }
        | ExprIr::LoadSlot { .. }
        | ExprIr::LoadConst { .. }
        | ExprIr::Field { .. }
        | ExprIr::MapLiteral { .. }
        | ExprIr::ArrayLiteral { .. }
        | ExprIr::Unary { .. }
        | ExprIr::Binary { .. }
        | ExprIr::Throw { .. }
        | ExprIr::Rethrow { .. }
        | ExprIr::ValueBlock { .. } => {}
    }

    // Constant expressions may have been lowered before references were finalized.
    for service_symbol in &previous_refs.service_symbols {
        push_unique(&mut refs.service_symbols, service_symbol.clone());
    }
    for service_symbol in &previous_refs.service_dependency_symbols {
        push_unique(&mut refs.service_dependency_symbols, service_symbol.clone());
    }
    for package_symbol in &previous_refs.package_symbols {
        push_unique(&mut refs.package_symbols, package_symbol.clone());
    }
    for package_operation in &previous_refs.package_operation_symbols {
        push_unique(
            &mut refs.package_operation_symbols,
            package_operation.clone(),
        );
    }
    for native_target in &previous_refs.native_targets {
        push_unique(&mut refs.native_targets, native_target.clone());
    }
}

fn collect_metadata_external_refs(metadata: &MetadataValue, refs: &mut ExternalRefTable) {
    match metadata {
        MetadataValue::Array(items) => {
            for item in items {
                collect_metadata_external_refs(item, refs);
            }
        }
        MetadataValue::Object(entries) => {
            for value in entries.values() {
                collect_metadata_external_refs(value, refs);
            }
        }
        MetadataValue::Null
        | MetadataValue::Bool(_)
        | MetadataValue::Number(_)
        | MetadataValue::String(_) => {}
    }
}

fn collect_call_target_external_refs(target: &CallTargetIr, refs: &mut ExternalRefTable) {
    match target {
        CallTargetIr::ExternalServiceSymbol { symbol } => {
            push_unique(&mut refs.service_symbols, symbol.clone());
        }
        CallTargetIr::ServiceDependencySymbol { symbol } => {
            push_unique(&mut refs.service_dependency_symbols, symbol.clone());
        }
        CallTargetIr::PackageSymbol {
            package_ref,
            operation,
        } => {
            push_unique(
                &mut refs.package_operation_symbols,
                PackageOperationSymbolRef {
                    package_ref: package_ref.clone(),
                    operation: operation.clone(),
                },
            );
        }
        CallTargetIr::Native { target } => {
            push_unique(&mut refs.native_targets, target.clone());
        }
        CallTargetIr::InterfaceMethod { interface, .. } => {
            for arg in &interface.canonical_type_args {
                collect_type_ref_external_refs(arg, refs);
            }
        }
        CallTargetIr::LocalExecutable { .. }
        | CallTargetIr::PublicationExecutable { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. } => {}
    }
}

fn collect_pattern_external_refs(pattern: &PatternIr, refs: &mut ExternalRefTable) {
    match pattern {
        PatternIr::Type { ty } => collect_type_ref_external_refs(ty, refs),
        PatternIr::Wildcard | PatternIr::Literal { .. } | PatternIr::Binding { .. } => {}
    }
}

fn collect_type_ref_external_refs_from_descriptor(
    descriptor: &TypeDescriptorIr,
    refs: &mut ExternalRefTable,
) {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for field in fields.values() {
                collect_type_ref_external_refs(field, refs);
            }
        }
        TypeDescriptorIr::Union { variants } => {
            for variant in variants {
                collect_type_ref_external_refs(variant, refs);
            }
        }
        TypeDescriptorIr::Alias { target } => collect_type_ref_external_refs(target, refs),
        TypeDescriptorIr::Native { .. } => {}
    }
}

fn collect_type_ref_external_refs(ty: &TypeRefIr, refs: &mut ExternalRefTable) {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            push_unique(&mut refs.package_symbols, symbol.clone())
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            push_unique(&mut refs.service_symbols, symbol.clone());
        }
        TypeRefIr::Native { args, .. } => {
            for arg in args {
                collect_type_ref_external_refs(arg, refs);
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                collect_type_ref_external_refs(field, refs);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_type_ref_external_refs(item, refs);
            }
        }
        TypeRefIr::Nullable { inner } => collect_type_ref_external_refs(inner, refs),
        TypeRefIr::AnyInterface { interface } => {
            for arg in &interface.canonical_type_args {
                collect_type_ref_external_refs(arg, refs);
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                collect_type_ref_external_refs(&param.ty, refs);
            }
            collect_type_ref_external_refs(return_type, refs);
        }
        TypeRefIr::LocalType { .. } | TypeRefIr::Literal { .. } | TypeRefIr::TypeParam { .. } => {}
        TypeRefIr::PublicationType { .. } => {}
    }
}

fn collect_box_source_external_refs(source: &BoxSourceIr, refs: &mut ExternalRefTable) {
    match source {
        BoxSourceIr::Local {
            concrete_type,
            method_table,
        } => {
            collect_type_ref_external_refs(concrete_type, refs);
            collect_type_ref_external_refs(&method_table.concrete_type, refs);
            for arg in &method_table.interface.canonical_type_args {
                collect_type_ref_external_refs(arg, refs);
            }
            for slot in &method_table.slots {
                for param in &slot.signature.params {
                    collect_type_ref_external_refs(&param.ty, refs);
                }
                collect_type_ref_external_refs(&slot.signature.return_type, refs);
            }
        }
        BoxSourceIr::Remote { .. } => {}
    }
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}
