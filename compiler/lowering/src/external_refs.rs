use std::collections::BTreeMap;

use crate::file_ir::{
    validate_file_ir_service_calls, AssignTargetIr, BoxSourceIr, CallIr, CallTargetIr,
    ExecutableBody, ExprIr, ExternalRefTable, FileIrServiceCallValidationError, FileIrUnit,
    InterfaceDeclIr, MetadataValue, PackageCallableRef, PatternIr, ServiceCallRefIndex, StmtIr,
    TypeDescriptorIr, TypeRefIr,
};
use skiff_artifact_model::{ServiceCallRef, RECEIVER_BUILTIN_CAPABILITY_VERSION};

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

/// Rebuilds all File IR external refs while preserving the semantic tuple
/// behind every owner-local ServiceCall index. Service-call refs are interned
/// in tuple order and every instruction index is rewritten to the new table.
pub(super) fn rebuild_external_refs_for_file_ir_unit(
    unit: &mut FileIrUnit,
) -> Result<(), FileIrServiceCallValidationError> {
    validate_file_ir_service_calls(unit)?;
    let previous_refs = std::mem::take(&mut unit.external_refs);
    let service_call_refs = previous_refs
        .service_call_refs
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let service_call_indices = service_call_refs
        .iter()
        .enumerate()
        .map(|(index, call_ref)| {
            (
                call_ref.clone(),
                ServiceCallRefIndex::try_from(index)
                    .expect("validated File IR service-call table length fits u32 indices"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    rewrite_service_call_indices(unit, &previous_refs, &service_call_indices);

    let mut refs = ExternalRefTable {
        service_call_refs,
        ..ExternalRefTable::default()
    };
    for ty in &unit.type_table {
        collect_type_ref_external_refs_from_descriptor(&ty.descriptor, &mut refs);
    }
    for interface in unit.declarations.interfaces.values() {
        collect_interface_external_refs(interface, &mut refs);
    }
    for constant in &unit.constants {
        collect_type_ref_external_refs(&constant.ty, &mut refs);
        collect_body_external_refs(&constant.body, &mut refs);
    }
    for executable in &unit.executables {
        for param in &executable.params {
            collect_type_ref_external_refs(&param.ty, &mut refs);
        }
        collect_type_ref_external_refs(&executable.return_type, &mut refs);
        if let Some(self_type) = &executable.self_type {
            collect_type_ref_external_refs(self_type, &mut refs);
        }
        collect_body_external_refs(&executable.body, &mut refs);
    }
    unit.external_refs = refs;
    validate_file_ir_service_calls(unit)
}

fn rewrite_service_call_indices(
    unit: &mut FileIrUnit,
    previous_refs: &ExternalRefTable,
    indices: &BTreeMap<ServiceCallRef, ServiceCallRefIndex>,
) {
    for body in unit
        .constants
        .iter_mut()
        .map(|constant| &mut constant.body)
        .chain(
            unit.executables
                .iter_mut()
                .map(|executable| &mut executable.body),
        )
    {
        for expression in &mut body.expressions {
            let ExprIr::Call { call } = expression else {
                continue;
            };
            let CallTargetIr::ServiceCall {
                service_call_ref_index,
            } = &mut call.target
            else {
                continue;
            };
            let call_ref = previous_refs
                .service_call_ref(*service_call_ref_index)
                .expect("service-call indices were validated before external-ref rebuild");
            *service_call_ref_index = indices[call_ref];
        }
    }
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

fn collect_body_external_refs(body: &ExecutableBody, refs: &mut ExternalRefTable) {
    for expr in &body.expressions {
        collect_expr_external_refs(expr, refs);
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

fn collect_expr_external_refs(expr: &ExprIr, refs: &mut ExternalRefTable) {
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
        CallTargetIr::ServiceCall { .. } => {}
        CallTargetIr::PackageCallable {
            package_ref,
            package_callable_id,
        } => {
            push_unique(
                &mut refs.package_callables,
                PackageCallableRef {
                    package_ref: package_ref.clone(),
                    package_callable_id: package_callable_id.clone(),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        validate_file_ir_service_calls, ContractOperationId, ServiceProtocolIdentity,
    };

    use super::*;

    #[test]
    fn rebuild_reinterns_service_call_tuples_and_rewrites_every_instruction_index() {
        let mut unit = FileIrUnit::empty("consumer.main", "source");
        let later = service_call_ref(1, "operation:zeta", "protocol:zeta");
        let earlier = service_call_ref(0, "operation:alpha", "protocol:alpha");
        unit.external_refs.service_call_refs = vec![later.clone(), earlier.clone()];
        unit.constants.push(skiff_artifact_model::ConstIr {
            name: "calls".to_string(),
            ty: TypeRefIr::builtin("void"),
            body: ExecutableBody {
                expressions: vec![service_call(0), service_call(1), service_call(0)],
                ..ExecutableBody::default()
            },
            source_span: None,
        });

        rebuild_external_refs_for_file_ir_unit(&mut unit).unwrap();

        assert_eq!(unit.external_refs.service_call_refs, vec![earlier, later]);
        assert_eq!(service_call_indices(&unit), vec![1, 0, 1]);
        validate_file_ir_service_calls(&unit).unwrap();
    }

    #[test]
    fn rebuild_rejects_invalid_service_call_tables_instead_of_dropping_them() {
        let mut unit = FileIrUnit::empty("consumer.main", "source");
        unit.external_refs.service_call_refs =
            vec![service_call_ref(0, "operation:orphan", "protocol:consumer")];
        assert!(matches!(
            rebuild_external_refs_for_file_ir_unit(&mut unit),
            Err(FileIrServiceCallValidationError::OrphanRef { .. })
        ));
    }

    fn service_call(index: u32) -> ExprIr {
        ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ServiceCall {
                    service_call_ref_index: ServiceCallRefIndex::new(index),
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        }
    }

    fn service_call_ref(slot: u32, operation: &str, protocol: &str) -> ServiceCallRef {
        ServiceCallRef {
            service_requirement_slot: slot,
            contract_operation_id: ContractOperationId::new(operation),
            expected_protocol_identity: ServiceProtocolIdentity::new(protocol),
        }
    }

    fn service_call_indices(unit: &FileIrUnit) -> Vec<u32> {
        unit.constants[0]
            .body
            .expressions
            .iter()
            .map(|expression| {
                let ExprIr::Call { call } = expression else {
                    panic!("service call expression")
                };
                let CallTargetIr::ServiceCall {
                    service_call_ref_index,
                } = call.target
                else {
                    panic!("canonical service call target")
                };
                service_call_ref_index.index()
            })
            .collect()
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
        TypeRefIr::Builtin { args, .. } => {
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
