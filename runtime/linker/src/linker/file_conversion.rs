use std::collections::BTreeMap;

use anyhow::Context;
use skiff_artifact_model as artifact;

use crate::program::{
    addr::{ExecutableIndex, TypeIndex},
    linked::*,
};

pub fn linked_file_unit_from_artifact(
    unit: &skiff_artifact_model::FileIrUnit,
) -> anyhow::Result<LinkedFileUnit> {
    if unit.required_receiver_builtin_capability_version > RECEIVER_BUILTIN_CAPABILITY_VERSION {
        anyhow::bail!(
            "File IR {} requires receiver builtin capability version {}, but runtime supports {}",
            unit.file_ir_identity,
            unit.required_receiver_builtin_capability_version,
            RECEIVER_BUILTIN_CAPABILITY_VERSION
        );
    }
    validate_receiver_builtin_ops(unit)?;
    Ok(LinkedFileUnit {
        schema_version: unit.schema_version.clone(),
        file_ir_identity: unit.file_ir_identity.clone(),
        source_ast_hash: unit.source_ast_hash.clone(),
        module_path: unit.module_path.clone(),
        ir_format_version: Some(unit.ir_format_version.clone()),
        opcode_table_version: Some(unit.opcode_table_version.clone()),
        source_map: linked_source_map(&unit.source_map)?,
        declarations: linked_declarations(&unit.declarations),
        link_targets: linked_link_targets(&unit.link_targets),
        types: unit.type_table.iter().map(linked_type_decl).collect(),
        constants: unit.constants.iter().map(linked_const).collect(),
        executables: unit.executables.iter().map(linked_executable).collect(),
        external_refs: linked_external_refs(&unit.external_refs),
    })
}

fn validate_receiver_builtin_ops(unit: &artifact::FileIrUnit) -> anyhow::Result<()> {
    for (const_index, constant) in unit.constants.iter().enumerate() {
        validate_receiver_builtin_ops_in_body(
            unit,
            &constant.body,
            format!("const[{const_index}] {}", constant.name),
        )?;
    }
    for (executable_index, executable) in unit.executables.iter().enumerate() {
        validate_receiver_builtin_ops_in_body(
            unit,
            &executable.body,
            format!("executable[{executable_index}] {}", executable.symbol),
        )?;
    }
    Ok(())
}

fn validate_receiver_builtin_ops_in_body(
    unit: &artifact::FileIrUnit,
    body: &artifact::ExecutableBody,
    owner: String,
) -> anyhow::Result<()> {
    for (expression_index, expression) in body.expressions.iter().enumerate() {
        let artifact::ExprIr::Call { call } = expression else {
            continue;
        };
        let artifact::CallTargetIr::ReceiverBuiltin { op } = &call.target else {
            continue;
        };
        artifact::validate_supported_receiver_builtin_op(op).map_err(|error| {
            anyhow::anyhow!(
                "File IR {} has unsupported receiver builtin op at {} expression[{}]: {}",
                unit.file_ir_identity,
                owner,
                expression_index,
                error
            )
        })?;
    }
    Ok(())
}

fn linked_source_map(source_map: &artifact::SourceMapDto) -> anyhow::Result<SourceMapDto> {
    Ok(SourceMapDto {
        format: Some(source_map.format.clone()),
        sources: source_map
            .sources
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to encode artifact source map sources")?,
        spans: source_map
            .spans
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to encode artifact source map spans")?,
    })
}

fn linked_declarations(declarations: &artifact::FileDeclarations) -> FileDeclarations {
    FileDeclarations {
        types: declarations
            .types
            .iter()
            .map(|(symbol, declaration)| (symbol.clone(), linked_type_declaration(declaration)))
            .collect(),
        interfaces: declarations
            .interfaces
            .iter()
            .map(|(symbol, declaration)| {
                (symbol.clone(), linked_interface_declaration(declaration))
            })
            .collect(),
        db: declarations
            .db
            .iter()
            .map(|(symbol, declaration)| (symbol.clone(), linked_db_declaration(declaration)))
            .collect(),
        executables: declarations
            .executables
            .iter()
            .map(|(symbol, declaration)| {
                (symbol.clone(), linked_executable_declaration(declaration))
            })
            .collect(),
        constants: declarations
            .constants
            .iter()
            .map(|(symbol, declaration)| (symbol.clone(), linked_const_declaration(declaration)))
            .collect(),
        symbols: BTreeMap::new(),
    }
}

fn linked_interface_declaration(declaration: &artifact::InterfaceDeclIr) -> InterfaceDeclIr {
    InterfaceDeclIr {
        name: declaration.name.clone(),
        type_params: declaration.type_params.clone(),
        operations: declaration
            .operations
            .iter()
            .map(linked_interface_operation)
            .collect(),
        source_span: declaration.source_span.clone(),
    }
}

fn linked_interface_operation(operation: &artifact::InterfaceOperationIr) -> InterfaceOperationIr {
    InterfaceOperationIr {
        name: operation.name.clone(),
        type_params: operation.type_params.clone(),
        params: operation
            .params
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: linked_type_ref(&param.ty),
            })
            .collect(),
        return_type: linked_type_ref(&operation.return_type),
        is_native: operation.is_native,
        is_provider: operation.is_provider,
        is_static: operation.is_static,
        implicit_self: operation.implicit_self.as_ref().map(linked_type_ref),
    }
}

fn linked_type_declaration(declaration: &artifact::TypeDeclarationIr) -> TypeDeclarationIr {
    TypeDeclarationIr {
        type_index: declaration.type_index as TypeIndex,
        symbol: declaration.symbol.clone(),
        source_span: declaration.source_span.clone(),
    }
}

fn linked_executable_declaration(
    declaration: &artifact::ExecutableDeclarationIr,
) -> ExecutableDeclarationIr {
    ExecutableDeclarationIr {
        executable_index: declaration.executable_index as ExecutableIndex,
        symbol: declaration.symbol.clone(),
        source_span: declaration.source_span.clone(),
    }
}

fn linked_const_declaration(declaration: &artifact::ConstDeclarationIr) -> ConstDeclarationIr {
    ConstDeclarationIr {
        const_index: declaration.const_index as ConstIndex,
        symbol: declaration.symbol.clone(),
        ty: linked_type_ref(&declaration.ty),
        source_span: declaration.source_span.clone(),
    }
}

fn linked_db_declaration(declaration: &artifact::DbDeclarationIr) -> DbDeclarationIr {
    DbDeclarationIr {
        type_ref: linked_type_ref(&declaration.type_ref),
        type_name: declaration.type_name.clone(),
        collection_name: declaration.collection_name.clone(),
        kind: match declaration.kind {
            artifact::DbObjectKindIr::Object => DbObjectKindIr::Object,
        },
        key: DbObjectKeyIr {
            name: declaration.key.name.clone(),
            ty: linked_type_ref(&declaration.key.ty),
        },
        fields: declaration
            .fields
            .iter()
            .map(|field| DbObjectFieldIr {
                name: field.name.clone(),
                ty: linked_type_ref(&field.ty),
            })
            .collect(),
        leases: declaration
            .leases
            .iter()
            .map(|lease| DbLeaseIr {
                name: lease.name.clone(),
                ttl_ms: lease.ttl_ms,
                max_ms: lease.max_ms,
            })
            .collect(),
        indexes: declaration
            .indexes
            .iter()
            .map(|index| DbIndexIr {
                name: index.name.clone(),
                unique: index.unique,
                fields: index
                    .fields
                    .iter()
                    .map(|field| DbIndexFieldIr {
                        field: linked_field_path(&field.field),
                        direction: linked_db_index_direction(field.direction),
                    })
                    .collect(),
                where_expr: index.where_expr.clone(),
            })
            .collect(),
        source_span: declaration.source_span.clone(),
    }
}

fn linked_db_index_direction(direction: artifact::DbIndexDirectionIr) -> DbIndexDirectionIr {
    match direction {
        artifact::DbIndexDirectionIr::Asc => DbIndexDirectionIr::Asc,
        artifact::DbIndexDirectionIr::Desc => DbIndexDirectionIr::Desc,
    }
}

fn linked_link_targets(link_targets: &artifact::FileLinkTargets) -> FileLinkTargets {
    FileLinkTargets {
        types: link_targets
            .types
            .iter()
            .map(|(symbol, export)| (symbol.clone(), export.type_index as TypeIndex))
            .collect(),
        executables: link_targets
            .executables
            .iter()
            .map(|(symbol, export)| (symbol.clone(), export.executable_index as ExecutableIndex))
            .collect(),
        constants: link_targets
            .constants
            .iter()
            .map(|(symbol, export)| (symbol.clone(), export.const_index as ConstIndex))
            .collect(),
    }
}

fn linked_type_decl(declaration: &artifact::TypeDeclIr) -> TypeDeclIr {
    TypeDeclIr {
        name: declaration.name.clone(),
        descriptor: match &declaration.descriptor {
            artifact::TypeDescriptorIr::Record { fields } => LinkedTypeDescriptor::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), linked_type_ref(ty)))
                    .collect(),
            },
            artifact::TypeDescriptorIr::Alias { target } => LinkedTypeDescriptor::Alias {
                target: linked_type_ref(target),
            },
            artifact::TypeDescriptorIr::Union { variants } => LinkedTypeDescriptor::Union {
                variants: variants.iter().map(linked_type_ref).collect(),
            },
            artifact::TypeDescriptorIr::Native { symbol } => LinkedTypeDescriptor::Native {
                symbol: symbol.clone(),
            },
        },
        type_params: declaration.type_params.clone(),
        discriminator: declaration.discriminator.clone(),
        implements: declaration.implements.iter().map(linked_type_ref).collect(),
        source_span: declaration.source_span.clone(),
    }
}

fn linked_const(constant: &artifact::ConstIr) -> ConstIr {
    ConstIr {
        name: constant.name.clone(),
        ty: linked_type_ref(&constant.ty),
        body: linked_body(&constant.body),
        source_span: constant.source_span.clone(),
    }
}

fn linked_external_refs(external_refs: &artifact::ExternalRefTable) -> ExternalRefTable {
    ExternalRefTable {
        service_symbols: external_refs.service_symbols.clone(),
        service_dependency_symbols: external_refs.service_dependency_symbols.clone(),
        package_symbols: external_refs.package_symbols.clone(),
        package_operation_symbols: external_refs.package_operation_symbols.clone(),
        native_targets: external_refs.native_targets.clone(),
        refs: BTreeMap::new(),
    }
}

fn linked_executable(executable: &artifact::ExecutableIr) -> LinkedExecutable {
    LinkedExecutable {
        kind: match executable.kind {
            artifact::ExecutableKind::Function => ExecutableKind::Function,
            artifact::ExecutableKind::ImplMethod => ExecutableKind::ImplMethod,
        },
        symbol: executable.symbol.clone(),
        type_params: executable.type_params.clone(),
        params: executable
            .params
            .iter()
            .map(|param| ParamIr {
                name: param.name.clone(),
                slot: param.slot as usize,
                ty: linked_type_ref(&param.ty),
            })
            .collect(),
        return_type: Some(linked_type_ref(&executable.return_type)),
        self_type: executable.self_type.as_ref().map(linked_type_ref),
        slots: SlotLayoutIr {
            slots: executable
                .slots
                .slots
                .iter()
                .map(|slot| SlotIr {
                    index: slot.index as usize,
                    name: slot.name.clone(),
                    kind: linked_slot_kind(slot.kind).to_string(),
                })
                .collect(),
            frame_size: executable.slots.frame_size as usize,
        },
        may_suspend: executable.may_suspend,
        body: linked_body(&executable.body),
    }
}

fn linked_slot_kind(kind: artifact::SlotKind) -> &'static str {
    match kind {
        artifact::SlotKind::Param => "param",
        artifact::SlotKind::SelfValue => "selfValue",
        artifact::SlotKind::Local => "local",
        artifact::SlotKind::Temp => "temp",
        artifact::SlotKind::Pattern => "pattern",
    }
}

fn linked_body(body: &artifact::ExecutableBody) -> LinkedExecutableBody {
    LinkedExecutableBody {
        blocks: body
            .blocks
            .iter()
            .map(|block| BlockIr {
                label: block.label.clone(),
                statements: block.statements.iter().map(linked_stmt_ref).collect(),
            })
            .collect(),
        statements: body.statements.iter().map(linked_stmt).collect(),
        expressions: body.expressions.iter().map(linked_expr).collect(),
    }
}

fn linked_expr_ref(reference: &artifact::ExprRefIr) -> ExprRefIr {
    ExprRefIr {
        expression: reference.expression,
    }
}

fn linked_stmt_ref(reference: &artifact::StmtRefIr) -> StmtRefIr {
    StmtRefIr {
        statement: reference.statement,
    }
}

fn linked_stmt(statement: &artifact::StmtIr) -> LinkedStmtIr {
    match statement {
        artifact::StmtIr::Let { slot, value } => LinkedStmtIr::Let {
            slot: *slot,
            value: linked_expr_ref(value),
        },
        artifact::StmtIr::Assign { target, value } => LinkedStmtIr::Assign {
            target: linked_assign_target(target),
            value: linked_expr_ref(value),
        },
        artifact::StmtIr::If {
            condition,
            then_block,
            else_block,
        } => LinkedStmtIr::If {
            condition: linked_expr_ref(condition),
            then_block: then_block.clone(),
            else_block: else_block.clone(),
        },
        artifact::StmtIr::ForIn {
            item_slot,
            item_type,
            value_slot,
            iterable,
            body,
        } => LinkedStmtIr::ForIn {
            item_slot: *item_slot,
            item_type: item_type.as_ref().map(linked_type_ref),
            value_slot: *value_slot,
            iterable: linked_expr_ref(iterable),
            body: body.clone(),
        },
        artifact::StmtIr::Match { value, arms } => LinkedStmtIr::Match {
            value: linked_expr_ref(value),
            arms: arms
                .iter()
                .map(|arm| MatchArmIr {
                    pattern: linked_pattern(&arm.pattern),
                    body: arm.body.clone(),
                })
                .collect(),
        },
        artifact::StmtIr::Assert { condition, message } => LinkedStmtIr::Assert {
            condition: linked_expr_ref(condition),
            message: message.as_ref().map(linked_expr_ref),
        },
        artifact::StmtIr::Break => LinkedStmtIr::Break,
        artifact::StmtIr::Continue => LinkedStmtIr::Continue,
        artifact::StmtIr::Spawn { call } => LinkedStmtIr::Spawn {
            call: linked_expr_ref(call),
        },
        artifact::StmtIr::Emit { operation, value } => LinkedStmtIr::Emit {
            operation: operation.clone(),
            value: linked_expr_ref(value),
        },
        artifact::StmtIr::Expr { value } => LinkedStmtIr::Expr {
            value: linked_expr_ref(value),
        },
        artifact::StmtIr::Return { value } => LinkedStmtIr::Return {
            value: value.as_ref().map(linked_expr_ref),
        },
        artifact::StmtIr::Throw {
            value,
            payload_type,
        } => LinkedStmtIr::Throw {
            value: linked_expr_ref(value),
            payload_type: linked_type_ref(payload_type),
        },
        artifact::StmtIr::Rethrow { exception_slot } => LinkedStmtIr::Rethrow {
            exception_slot: *exception_slot,
        },
    }
}

fn linked_assign_target(target: &artifact::AssignTargetIr) -> AssignTargetIr {
    match target {
        artifact::AssignTargetIr::Slot { slot } => AssignTargetIr::Slot { slot: *slot },
        artifact::AssignTargetIr::Field { object, field } => AssignTargetIr::Field {
            object: linked_expr_ref(object),
            field: field.clone(),
        },
        artifact::AssignTargetIr::Index { object, index } => AssignTargetIr::Index {
            object: linked_expr_ref(object),
            index: linked_expr_ref(index),
        },
    }
}

fn linked_pattern(pattern: &artifact::PatternIr) -> PatternIr {
    match pattern {
        artifact::PatternIr::Wildcard => PatternIr::Wildcard,
        artifact::PatternIr::Literal { value } => PatternIr::Literal {
            value: value.clone(),
        },
        artifact::PatternIr::Type { ty } => PatternIr::Type {
            ty: linked_type_ref(ty),
        },
        artifact::PatternIr::Binding { slot } => PatternIr::Binding { slot: *slot },
    }
}

fn linked_expr(expression: &artifact::ExprIr) -> LinkedExprIr {
    match expression {
        artifact::ExprIr::Literal { value } => LinkedExprIr::Literal {
            value: value.clone(),
        },
        artifact::ExprIr::LoadSlot { slot } => LinkedExprIr::LoadSlot { slot: *slot },
        artifact::ExprIr::LoadConst { const_index } => LinkedExprIr::LoadConst {
            const_index: *const_index,
        },
        artifact::ExprIr::Field { object, field } => LinkedExprIr::Field {
            object: linked_expr_ref(object),
            field: field.clone(),
        },
        artifact::ExprIr::Construct { type_ref, fields } => LinkedExprIr::Construct {
            type_ref: linked_type_ref(type_ref),
            fields: linked_expr_ref_map(fields),
        },
        artifact::ExprIr::InterfaceBox {
            value,
            interface,
            source,
        } => LinkedExprIr::InterfaceBox {
            value: linked_expr_ref(value),
            interface: linked_interface_instantiation_ref(interface),
            source: linked_box_source(source),
        },
        artifact::ExprIr::MapLiteral { entries } => LinkedExprIr::MapLiteral {
            entries: linked_expr_ref_map(entries),
        },
        artifact::ExprIr::ArrayLiteral { items } => LinkedExprIr::ArrayLiteral {
            items: items.iter().map(linked_expr_ref).collect(),
        },
        artifact::ExprIr::Unary { op, value } => LinkedExprIr::Unary {
            op: linked_unary_op(*op),
            value: linked_expr_ref(value),
        },
        artifact::ExprIr::Binary { op, left, right } => LinkedExprIr::Binary {
            op: linked_binary_op(*op),
            left: linked_expr_ref(left),
            right: linked_expr_ref(right),
        },
        artifact::ExprIr::Call { call } => LinkedExprIr::Call {
            call: linked_call(call),
        },
        artifact::ExprIr::Throw {
            value,
            payload_type,
        } => LinkedExprIr::Throw {
            value: linked_expr_ref(value),
            payload_type: linked_type_ref(payload_type),
        },
        artifact::ExprIr::Rethrow { exception_slot } => LinkedExprIr::Rethrow {
            exception_slot: *exception_slot,
        },
        artifact::ExprIr::Catch {
            try_expression,
            catch_slot,
            catch_type,
            body,
        } => LinkedExprIr::Catch {
            try_expression: linked_expr_ref(try_expression),
            catch_slot: *catch_slot,
            catch_type: catch_type.as_ref().map(linked_type_ref),
            body: linked_expr_ref(body),
        },
        artifact::ExprIr::ValueBlock { block, result } => LinkedExprIr::ValueBlock {
            block: block.clone(),
            result: linked_expr_ref(result),
        },
        artifact::ExprIr::DbOperation { operation } => LinkedExprIr::DbOperation {
            operation: linked_db_operation(operation),
        },
        artifact::ExprIr::DbQuery { query } => LinkedExprIr::DbQuery {
            target: linked_db_target(&query.target),
            query: linked_db_query(&query.query),
            projection: None,
            result_type: Some(linked_type_ref(&query.result_type)),
        },
        artifact::ExprIr::DbTransaction { transaction } => LinkedExprIr::DbTransaction {
            transaction: linked_db_transaction(transaction),
        },
        artifact::ExprIr::DbLeaseClaim { claim } => LinkedExprIr::DbLeaseClaim {
            claim: linked_db_lease_claim(claim),
        },
        artifact::ExprIr::DbLeaseRead { read } => LinkedExprIr::DbLeaseRead {
            read: linked_db_lease_read(read),
        },
    }
}

fn linked_expr_ref_map(map: &BTreeMap<String, artifact::ExprRefIr>) -> BTreeMap<String, ExprRefIr> {
    map.iter()
        .map(|(key, value)| (key.clone(), linked_expr_ref(value)))
        .collect()
}

fn linked_db_operation(operation: &artifact::DbOperationIr) -> DbOperationIr {
    DbOperationIr {
        op: linked_db_op_kind(operation.op),
        many: operation.many,
        target: linked_db_target(&operation.target),
        selector: operation.selector.as_ref().map(linked_db_selector),
        query: operation.query.as_ref().map(linked_db_query),
        projection: operation.projection.as_ref().map(linked_db_projection),
        body: operation.body.as_ref().map(linked_db_body),
        insert_body: operation.insert_body.as_ref().map(linked_db_body),
        change: operation.change.as_ref().map(linked_db_change),
        result_type: linked_type_ref(&operation.result_type),
        source_span: operation.source_span.clone(),
    }
}

fn linked_db_op_kind(kind: artifact::DbOpKindIr) -> DbOpKindIr {
    match kind {
        artifact::DbOpKindIr::Find => DbOpKindIr::Find,
        artifact::DbOpKindIr::Optional => DbOpKindIr::Optional,
        artifact::DbOpKindIr::Require => DbOpKindIr::Require,
        artifact::DbOpKindIr::Insert => DbOpKindIr::Insert,
        artifact::DbOpKindIr::Update => DbOpKindIr::Update,
        artifact::DbOpKindIr::Upsert => DbOpKindIr::Upsert,
        artifact::DbOpKindIr::Replace => DbOpKindIr::Replace,
        artifact::DbOpKindIr::Delete => DbOpKindIr::Delete,
        artifact::DbOpKindIr::Count => DbOpKindIr::Count,
        artifact::DbOpKindIr::Exists => DbOpKindIr::Exists,
    }
}

fn linked_db_target(target: &artifact::DbTargetIr) -> DbTargetIr {
    DbTargetIr {
        type_ref: linked_type_ref(&target.type_ref),
        type_name: target.type_name.clone(),
    }
}

fn linked_db_selector(selector: &artifact::DbSelectorIr) -> DbSelectorIr {
    match selector {
        artifact::DbSelectorIr::Key { value } => DbSelectorIr::Key {
            value: linked_expr_ref(value),
        },
        artifact::DbSelectorIr::Query { query } => DbSelectorIr::Query {
            query: linked_db_query(query),
        },
    }
}

fn linked_db_query(query: &artifact::DbQueryIr) -> DbQueryIr {
    DbQueryIr {
        where_: query
            .where_clauses
            .iter()
            .map(linked_db_predicate)
            .collect(),
        order: query
            .order
            .iter()
            .map(|order| DbOrderIr {
                field: linked_field_path(&order.field),
                direction: linked_db_index_direction(order.direction),
            })
            .collect(),
        limit: query.limit.as_ref().map(linked_expr_ref),
        offset: query.offset.as_ref().map(linked_expr_ref),
        after: query.after.as_ref().map(linked_expr_ref),
    }
}

fn linked_db_predicate(predicate: &artifact::DbPredicateIr) -> DbPredicateIr {
    match predicate {
        artifact::DbPredicateIr::Compare { field, op, value } => DbPredicateIr::Compare {
            field: linked_field_path(field),
            op: linked_db_predicate_compare_op(*op),
            value: linked_expr_ref(value),
        },
        artifact::DbPredicateIr::Regex {
            field,
            pattern,
            options,
        } => DbPredicateIr::Regex {
            field: linked_field_path(field),
            pattern: linked_expr_ref(pattern),
            options: options.as_ref().map(linked_expr_ref),
        },
        artifact::DbPredicateIr::And { predicates } => DbPredicateIr::And {
            predicates: predicates.iter().map(linked_db_predicate).collect(),
        },
        artifact::DbPredicateIr::Or { predicates } => DbPredicateIr::Or {
            predicates: predicates.iter().map(linked_db_predicate).collect(),
        },
        artifact::DbPredicateIr::Not { predicate } => DbPredicateIr::Not {
            predicate: Box::new(linked_db_predicate(predicate)),
        },
        artifact::DbPredicateIr::Conditional {
            condition,
            predicate,
        } => DbPredicateIr::Conditional {
            condition: linked_expr_ref(condition),
            predicate: Box::new(linked_db_predicate(predicate)),
        },
    }
}

fn linked_db_predicate_compare_op(op: artifact::DbPredicateCompareOpIr) -> DbPredicateCompareOpIr {
    match op {
        artifact::DbPredicateCompareOpIr::Eq => DbPredicateCompareOpIr::Eq,
        artifact::DbPredicateCompareOpIr::Ne => DbPredicateCompareOpIr::Ne,
        artifact::DbPredicateCompareOpIr::Lt => DbPredicateCompareOpIr::Lt,
        artifact::DbPredicateCompareOpIr::Lte => DbPredicateCompareOpIr::Lte,
        artifact::DbPredicateCompareOpIr::Gt => DbPredicateCompareOpIr::Gt,
        artifact::DbPredicateCompareOpIr::Gte => DbPredicateCompareOpIr::Gte,
    }
}

fn linked_db_projection(projection: &artifact::DbProjectionIr) -> DbProjectionIr {
    DbProjectionIr {
        fields: projection.fields.iter().map(linked_field_path).collect(),
    }
}

fn linked_db_body(body: &artifact::DbBodyIr) -> DbBodyIr {
    match body {
        artifact::DbBodyIr::ObjectFields { fields } => DbBodyIr::ObjectFields {
            fields: linked_expr_ref_map(fields),
        },
        artifact::DbBodyIr::Values { value } => DbBodyIr::Values {
            value: linked_expr_ref(value),
        },
    }
}

fn linked_db_change(change: &artifact::DbChangeIr) -> DbChangeIr {
    DbChangeIr {
        ops: change.ops.iter().map(linked_db_change_op).collect(),
    }
}

fn linked_db_change_op(op: &artifact::DbChangeOpIr) -> DbChangeOpIr {
    match op {
        artifact::DbChangeOpIr::Set { path, value } => DbChangeOpIr::Set {
            field: linked_field_path(path),
            value: linked_expr_ref(value),
        },
        artifact::DbChangeOpIr::Inc { path, value } => DbChangeOpIr::Inc {
            field: linked_field_path(path),
            value: linked_expr_ref(value),
        },
        artifact::DbChangeOpIr::Unset { path } => DbChangeOpIr::Unset {
            field: linked_field_path(path),
        },
        artifact::DbChangeOpIr::AddToSet { path, value } => DbChangeOpIr::AddToSet {
            field: linked_field_path(path),
            value: linked_expr_ref(value),
        },
        artifact::DbChangeOpIr::Remove { path, value } => DbChangeOpIr::Remove {
            field: linked_field_path(path),
            value: linked_expr_ref(value),
        },
    }
}

fn linked_db_transaction(transaction: &artifact::DbTransactionIr) -> DbTransactionIr {
    DbTransactionIr {
        mode: match transaction.mode {
            artifact::DbBlockModeIr::Effect => DbTransactionModeIr::Effect,
            artifact::DbBlockModeIr::Value => DbTransactionModeIr::Value,
        },
        body: transaction.body.clone(),
        result: Some(linked_expr_ref(&transaction.result)),
        result_type: linked_type_ref(&transaction.result_type),
    }
}

fn linked_db_lease_claim(claim: &artifact::DbLeaseClaimIr) -> DbLeaseClaimIr {
    DbLeaseClaimIr {
        target: linked_db_target(&claim.target),
        key: linked_expr_ref(&claim.key),
        slot: claim.slot.clone(),
        binding_slot: claim.binding_slot,
        body: claim.body.clone(),
        result_type: linked_type_ref(&claim.result_type),
        source_span: claim.source_span.clone(),
    }
}

fn linked_db_lease_read(read: &artifact::DbLeaseReadIr) -> DbLeaseReadIr {
    DbLeaseReadIr {
        target: linked_db_target(&read.target),
        key: linked_expr_ref(&read.key),
        slot: read.slot.clone(),
        result_type: linked_type_ref(&read.result_type),
        source_span: read.source_span.clone(),
    }
}

fn linked_unary_op(op: artifact::UnaryOpIr) -> UnaryOpIr {
    match op {
        artifact::UnaryOpIr::Not => UnaryOpIr::Not,
        artifact::UnaryOpIr::Negate => UnaryOpIr::Negate,
    }
}

fn linked_binary_op(op: artifact::BinaryOpIr) -> BinaryOpIr {
    match op {
        artifact::BinaryOpIr::Add => BinaryOpIr::Add,
        artifact::BinaryOpIr::Subtract => BinaryOpIr::Subtract,
        artifact::BinaryOpIr::Multiply => BinaryOpIr::Multiply,
        artifact::BinaryOpIr::Divide => BinaryOpIr::Divide,
        artifact::BinaryOpIr::Equal => BinaryOpIr::Equal,
        artifact::BinaryOpIr::NotEqual => BinaryOpIr::NotEqual,
        artifact::BinaryOpIr::LessThan => BinaryOpIr::LessThan,
        artifact::BinaryOpIr::LessThanOrEqual => BinaryOpIr::LessThanOrEqual,
        artifact::BinaryOpIr::GreaterThan => BinaryOpIr::GreaterThan,
        artifact::BinaryOpIr::GreaterThanOrEqual => BinaryOpIr::GreaterThanOrEqual,
        artifact::BinaryOpIr::And => BinaryOpIr::And,
        artifact::BinaryOpIr::Or => BinaryOpIr::Or,
    }
}

fn linked_call(call: &artifact::CallIr) -> CallIr {
    CallIr {
        target: match &call.target {
            artifact::CallTargetIr::LocalExecutable { executable_index } => {
                LinkedCallTarget::LocalExecutable {
                    executable_index: *executable_index,
                }
            }
            artifact::CallTargetIr::ExternalServiceSymbol { symbol } => {
                LinkedCallTarget::ExternalServiceSymbol {
                    symbol: symbol.clone(),
                }
            }
            artifact::CallTargetIr::ServiceDependencySymbol { symbol } => {
                LinkedCallTarget::ServiceDependencySymbol {
                    symbol: symbol.clone(),
                }
            }
            artifact::CallTargetIr::PackageSymbol {
                package_ref,
                operation,
            } => LinkedCallTarget::PackageSymbol {
                package_ref: package_ref.clone(),
                operation: operation.clone(),
            },
            artifact::CallTargetIr::Native { target } => LinkedCallTarget::Native {
                target: target.clone(),
            },
            artifact::CallTargetIr::Builtin { op } => LinkedCallTarget::Builtin { op: op.clone() },
            artifact::CallTargetIr::ReceiverBuiltin { op } => {
                LinkedCallTarget::ReceiverBuiltin { op: *op }
            }
            artifact::CallTargetIr::InterfaceMethod {
                interface,
                method_abi_id,
                slot,
            } => LinkedCallTarget::InterfaceMethod {
                interface: linked_interface_instantiation_ref(interface),
                method_abi_id: method_abi_id.clone(),
                slot: *slot,
            },
        },
        args: call.args.iter().map(linked_expr_ref).collect(),
        type_args: call
            .type_args
            .iter()
            .map(|(name, ty)| (name.clone(), linked_type_ref(ty)))
            .collect(),
        metadata: call.metadata.clone(),
    }
}

fn linked_field_path(path: &artifact::FieldPathIr) -> FieldPathIr {
    FieldPathIr {
        text: path.text.clone(),
        segments: path.segments.clone(),
    }
}

fn linked_type_ref(ty: &artifact::TypeRefIr) -> LinkedTypeRef {
    match ty {
        artifact::TypeRefIr::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args.iter().map(linked_type_ref).collect(),
        },
        artifact::TypeRefIr::LocalType { type_index } => LinkedTypeRef::LocalType {
            type_index: *type_index as TypeIndex,
        },
        artifact::TypeRefIr::ServiceSymbol { symbol } => LinkedTypeRef::ServiceSymbol {
            symbol: symbol.clone(),
        },
        artifact::TypeRefIr::PackageSymbol { symbol } => LinkedTypeRef::PackageSymbol {
            symbol: symbol.clone(),
        },
        artifact::TypeRefIr::DbObjectSymbol { symbol } => LinkedTypeRef::DbObjectSymbol {
            symbol: symbol.clone(),
        },
        artifact::TypeRefIr::Record { fields } => LinkedTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), linked_type_ref(ty)))
                .collect(),
        },
        artifact::TypeRefIr::Union { items } => LinkedTypeRef::Union {
            items: items.iter().map(linked_type_ref).collect(),
        },
        artifact::TypeRefIr::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(linked_type_ref(inner)),
        },
        artifact::TypeRefIr::Literal { value } => LinkedTypeRef::Literal {
            value: value.clone(),
        },
        artifact::TypeRefIr::TypeParam { name } => LinkedTypeRef::TypeParam { name: name.clone() },
        artifact::TypeRefIr::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: linked_interface_instantiation_ref(interface),
        },
        artifact::TypeRefIr::Function {
            params,
            return_type,
        } => LinkedTypeRef::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: linked_type_ref(&param.ty),
                })
                .collect(),
            return_type: Box::new(linked_type_ref(return_type)),
        },
    }
}

fn linked_interface_instantiation_ref(
    interface: &artifact::InterfaceInstantiationRef,
) -> LinkedInterfaceInstantiationRef {
    LinkedInterfaceInstantiationRef {
        interface_abi_id: interface.interface_abi_id.clone(),
        canonical_type_args: interface
            .canonical_type_args
            .iter()
            .map(linked_type_ref)
            .collect(),
    }
}

fn linked_box_source(source: &artifact::BoxSourceIr) -> LinkedBoxSourceIr {
    match source {
        artifact::BoxSourceIr::Local {
            concrete_type,
            method_table,
        } => LinkedBoxSourceIr::Local {
            concrete_type: linked_type_ref(concrete_type),
            method_table: linked_interface_method_table_plan(method_table),
        },
        artifact::BoxSourceIr::Remote {
            dependency_ref,
            public_instance_key,
            operations,
            callee_protocol_identity,
        } => LinkedBoxSourceIr::Remote {
            dependency_ref: dependency_ref.clone(),
            public_instance_key: public_instance_key.clone(),
            operations: linked_remote_operation_table_plan(operations),
            callee_protocol_identity: callee_protocol_identity.clone(),
        },
    }
}

fn linked_remote_operation_table_plan(
    plan: &artifact::RemoteOperationTablePlanIr,
) -> LinkedRemoteOperationTablePlanIr {
    LinkedRemoteOperationTablePlanIr {
        interface: linked_interface_instantiation_ref(&plan.interface),
        slots: plan
            .slots
            .iter()
            .map(linked_remote_operation_slot_plan)
            .collect(),
    }
}

fn linked_remote_operation_slot_plan(
    slot: &artifact::RemoteOperationSlotPlanIr,
) -> LinkedRemoteOperationSlotPlanIr {
    LinkedRemoteOperationSlotPlanIr {
        slot: slot.slot,
        method_abi_id: slot.method_abi_id.clone(),
        signature: LinkedInterfaceMethodSlotSignatureIr {
            params: slot
                .signature
                .params
                .iter()
                .map(|param| LinkedFunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: linked_type_ref(&param.ty),
                })
                .collect(),
            return_type: linked_type_ref(&slot.signature.return_type),
        },
        operation_abi_id: slot.operation_abi_id.clone(),
    }
}

fn linked_interface_method_table_plan(
    plan: &artifact::InterfaceMethodTablePlanIr,
) -> LinkedInterfaceMethodTablePlanIr {
    LinkedInterfaceMethodTablePlanIr {
        interface: linked_interface_instantiation_ref(&plan.interface),
        concrete_type: linked_type_ref(&plan.concrete_type),
        slots: plan
            .slots
            .iter()
            .map(linked_interface_method_slot_plan)
            .collect(),
    }
}

fn linked_interface_method_slot_plan(
    slot: &artifact::InterfaceMethodSlotPlanIr,
) -> LinkedInterfaceMethodSlotPlanIr {
    LinkedInterfaceMethodSlotPlanIr {
        slot: slot.slot,
        method_name: slot.method_name.clone(),
        method_abi_id: slot.method_abi_id.clone(),
        signature: LinkedInterfaceMethodSlotSignatureIr {
            params: slot
                .signature
                .params
                .iter()
                .map(|param| LinkedFunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: linked_type_ref(&param.ty),
                })
                .collect(),
            return_type: linked_type_ref(&slot.signature.return_type),
        },
        target: LinkedInterfaceMethodSlotTargetIr {
            executable_index: slot.target.executable_index,
            receiver_call_abi: slot.target.receiver_call_abi,
        },
    }
}
