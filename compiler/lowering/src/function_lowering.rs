use std::collections::{BTreeMap, BTreeSet};

use serde_json::Number;
use skiff_artifact_model::{builtin_receiver_op_by_name, BuiltinReceiverOp, ReceiverCallAbi};
use skiff_compiler_core::{
    id::SKIFF_STD_PUBLICATION_ID, package_export_resolver::PackageExportResolver,
};
use skiff_compiler_source::{
    prelude_registry::{prelude_registry, shared_native_alias_target},
    semantic::{executable_symbol, impl_method_declaration_name, InterfaceSemantics},
    ConstructorFieldValueSource, ExpressionKey, ExpressionOwnerKey, ExpressionTypeModel,
    LocalDbObjectIndex, PackageInterfaceMethodIndex, PublicationDbMetadataIndex,
    PublicationTypeSymbolIndex, ResolvedTypeRef, SourceSymbolKey, TypeResolutionContext,
    TypeResolutionModel,
};
use skiff_syntax::{
    ast::{
        BinaryOp, DbBlockMode, DbOperationKind, Expr, ForBinding, Literal, ObjectLiteralKey,
        PatchOperation, Stmt, TypeRef, UnaryOp,
    },
    error::{CompileError, Result},
    type_syntax::generic_parts,
};

use crate::file_ir::{
    AssignTargetIr, BinaryOpIr, BlockIr, BoxSourceIr, CallIr, CallTargetIr, ExecutableBody, ExprIr,
    ExprRefIr, FunctionTypeParamIr, InterfaceMethodSlotPlanIr, InterfaceMethodSlotSignatureIr,
    InterfaceMethodSlotTargetIr, InterfaceMethodTablePlanIr, LiteralIr, MatchArmIr, MetadataValue,
    NativeTarget, PackageRefIr, PackageSymbolRef, PatternIr, RemoteOperationSlotPlanIr,
    RemoteOperationTablePlanIr, ServiceDependencySymbolRef, ServiceSymbolRef, SlotIr, SlotKind,
    StmtIr, StmtRefIr, TypeRefIr, UnaryOpIr,
};

use super::{
    callable_return_types::CallableReturnType,
    db_lowering::{
        is_db_readonly_result_operation, DbMetadataIr, LoweredPublicationDbMetadataIndex,
    },
    dependency_operation_indexes::{PackageOperationIndex, ServiceDependencyOperationIndex},
    type_lowering::{
        is_official_std_module_path, is_unknown_type_ref, lower_named_type, lower_type_ref,
        lower_type_text, package_scoped_root_path, prelude_field_type_text,
        runtime_receiver_root_from_type_ref, service_symbol_ref, type_ref_ir_type_text,
        union_type_ir, TypeLoweringContext,
    },
};

const SPAWN_SUBMIT_METADATA_KEY: &str = "spawnSubmit";
const SPAWN_FUNCTION_TARGET_PREFIX: &str = "function:";

pub(super) fn native_target_from_symbol(symbol: &str) -> NativeTarget {
    let binding_key = prelude_registry()
        .native_binding_key(symbol)
        .map(std::string::ToString::to_string);
    if let Some((namespace, name)) = symbol.rsplit_once('.') {
        NativeTarget {
            namespace: namespace.to_string(),
            symbol: name.to_string(),
            binding_key,
            metadata: BTreeMap::new(),
        }
    } else {
        NativeTarget {
            namespace: String::new(),
            symbol: symbol.to_string(),
            binding_key,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Binding {
    slot: u32,
    mutable: bool,
    readonly: bool,
    readonly_array_item: bool,
    pub(super) type_text: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct BindingReadonlyFlags {
    pub(super) readonly: bool,
    pub(super) readonly_array_item: bool,
}

pub(super) struct FunctionLowerer<'a> {
    pub(super) type_indices: &'a BTreeMap<String, u32>,
    pub(super) package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub(super) db_metadata: &'a BTreeMap<String, DbMetadataIr>,
    pub(super) publication_db_metadata: &'a PublicationDbMetadataIndex,
    pub(super) lowered_publication_db_metadata: &'a LoweredPublicationDbMetadataIndex,
    pub(super) executable_indices: &'a BTreeMap<String, u32>,
    pub(super) const_indices: &'a BTreeMap<String, u32>,
    pub(super) external_type_symbols: &'a PublicationTypeSymbolIndex,
    pub(super) service_dependency_aliases: &'a BTreeSet<String>,
    pub(super) source_alias_targets: &'a BTreeMap<String, String>,
    pub(super) package_interface_methods: &'a PackageInterfaceMethodIndex,
    pub(super) package_operations: &'a PackageOperationIndex,
    pub(super) service_dependency_operations: &'a ServiceDependencyOperationIndex,
    pub(super) module_path: &'a str,
    pub(super) local_db_objects: &'a LocalDbObjectIndex,
    pub(super) type_param_scope: BTreeSet<String>,
    pub(super) expression_owner: Option<ExpressionOwnerKey>,
    pub(super) interface_semantics: &'a InterfaceSemantics,
    pub(super) type_resolution: &'a TypeResolutionModel,
    pub(super) expression_types: Option<&'a ExpressionTypeModel>,
    pub(super) callable_return_types: &'a BTreeMap<String, CallableReturnType>,
    pub(super) local_type_fields: &'a LocalTypeFieldIndex,
    executable_signatures: &'a BTreeMap<u32, LoweredExecutableSignature>,
    pub(super) next_expression_index: u32,
    pub(super) bindings: BTreeMap<String, Binding>,
    pub(super) scope_names: Vec<BTreeSet<String>>,
    pub(super) scope_binding_restore: Vec<Vec<(String, Option<Binding>)>>,
    pub(super) slots: Vec<SlotIr>,
    pub(super) body: ExecutableBody,
    pub(super) next_block_id: u32,
    pub(super) db_transaction_depth: u32,
}

pub(super) type LocalTypeFieldIndex = BTreeMap<u32, BTreeMap<String, TypeRefIr>>;

#[derive(Clone, Debug)]
pub(super) struct LoweredExecutableSignature {
    pub(super) params: Vec<FunctionTypeParamIr>,
    pub(super) return_type: TypeRefIr,
    pub(super) self_type: Option<TypeRefIr>,
}

impl<'a> FunctionLowerer<'a> {
    pub(super) fn new(
        type_indices: &'a BTreeMap<String, u32>,
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        db_metadata: &'a BTreeMap<String, DbMetadataIr>,
        publication_db_metadata: &'a PublicationDbMetadataIndex,
        lowered_publication_db_metadata: &'a LoweredPublicationDbMetadataIndex,
        executable_indices: &'a BTreeMap<String, u32>,
        const_indices: &'a BTreeMap<String, u32>,
        external_type_symbols: &'a PublicationTypeSymbolIndex,
        service_dependency_aliases: &'a BTreeSet<String>,
        source_alias_targets: &'a BTreeMap<String, String>,
        package_interface_methods: &'a PackageInterfaceMethodIndex,
        package_operations: &'a PackageOperationIndex,
        service_dependency_operations: &'a ServiceDependencyOperationIndex,
        module_path: &'a str,
        local_db_objects: &'a LocalDbObjectIndex,
        type_param_scope: BTreeSet<String>,
        expression_owner: Option<ExpressionOwnerKey>,
        interface_semantics: &'a InterfaceSemantics,
        type_resolution: &'a TypeResolutionModel,
        expression_types: Option<&'a ExpressionTypeModel>,
        callable_return_types: &'a BTreeMap<String, CallableReturnType>,
        local_type_fields: &'a LocalTypeFieldIndex,
        executable_signatures: &'a BTreeMap<u32, LoweredExecutableSignature>,
    ) -> Self {
        Self {
            type_indices,
            package_aliases,
            db_metadata,
            publication_db_metadata,
            lowered_publication_db_metadata,
            executable_indices,
            const_indices,
            external_type_symbols,
            service_dependency_aliases,
            source_alias_targets,
            package_interface_methods,
            package_operations,
            service_dependency_operations,
            module_path,
            local_db_objects,
            type_param_scope,
            expression_owner,
            interface_semantics,
            type_resolution,
            expression_types,
            callable_return_types,
            local_type_fields,
            executable_signatures,
            next_expression_index: 0,
            bindings: BTreeMap::new(),
            scope_names: vec![BTreeSet::new()],
            scope_binding_restore: vec![Vec::new()],
            slots: Vec::new(),
            body: ExecutableBody::default(),
            next_block_id: 0,
            db_transaction_depth: 0,
        }
    }

    pub(super) fn value_type_context(&self) -> TypeLoweringContext<'_> {
        TypeLoweringContext::value_with_type_params(&self.type_param_scope)
    }

    pub(super) fn db_target_type_context(&self) -> TypeLoweringContext<'_> {
        TypeLoweringContext::db_target_with_type_params(&self.type_param_scope)
    }

    pub(super) fn declare_slot(
        &mut self,
        name: &str,
        kind: SlotKind,
        mutable: bool,
    ) -> Result<u32> {
        self.declare_slot_with_type(name, kind, mutable, BindingReadonlyFlags::default(), None)
    }

    pub(super) fn declare_slot_with_type(
        &mut self,
        name: &str,
        kind: SlotKind,
        mutable: bool,
        readonly: BindingReadonlyFlags,
        type_text: Option<String>,
    ) -> Result<u32> {
        let current_scope = self
            .scope_names
            .last_mut()
            .expect("function lowerer always has at least one scope");
        if current_scope.contains(name) {
            return Err(CompileError::Semantic(format!(
                "duplicate binding `{name}` in File IR unit function"
            )));
        }
        let slot = self.slots.len() as u32;
        self.slots.push(SlotIr {
            index: slot,
            name: name.to_string(),
            kind,
        });
        current_scope.insert(name.to_string());
        let previous = self.bindings.insert(
            name.to_string(),
            Binding {
                slot,
                mutable,
                readonly: readonly.readonly,
                readonly_array_item: readonly.readonly_array_item,
                type_text,
            },
        );
        self.scope_binding_restore
            .last_mut()
            .expect("function lowerer always has at least one binding restore scope")
            .push((name.to_string(), previous));
        Ok(slot)
    }

    pub(super) fn push_scope(&mut self) {
        self.scope_names.push(BTreeSet::new());
        self.scope_binding_restore.push(Vec::new());
    }

    pub(super) fn pop_scope(&mut self) {
        let Some(_scope) = self.scope_names.pop() else {
            return;
        };
        let restore = self.scope_binding_restore.pop().unwrap_or_default();
        for (name, previous) in restore.into_iter().rev() {
            if let Some(previous) = previous {
                self.bindings.insert(name, previous);
            } else {
                self.bindings.remove(&name);
            }
        }
    }

    pub(super) fn next_block_label(&mut self, prefix: &str) -> String {
        let id = self.next_block_id;
        self.next_block_id += 1;
        format!("{prefix}${id}")
    }

    pub(super) fn lower_scoped_block<F>(
        &mut self,
        prefix: &str,
        block: &skiff_syntax::ast::Block,
        setup: F,
    ) -> Result<String>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        let label = self.next_block_label(prefix);
        self.push_scope();
        setup(self)?;
        let mut lowered = BlockIr {
            label: label.clone(),
            statements: Vec::new(),
        };
        for stmt in &block.statements {
            lowered.statements.push(self.lower_stmt(stmt)?);
        }
        self.pop_scope();
        self.body.blocks.push(lowered);
        Ok(label)
    }

    pub(super) fn lower_stmt(&mut self, stmt: &Stmt) -> Result<StmtRefIr> {
        let lowered = match stmt {
            Stmt::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let readonly = self.readonly_flags_for_value(value);
                let type_text = ty
                    .as_ref()
                    .map(|ty| ty.name.clone())
                    .or_else(|| self.next_expression_type_text())
                    .or_else(|| self.infer_expr_type_text(value));
                let value = self.lower_expr(value)?;
                let slot = self.declare_slot_with_type(
                    name,
                    SlotKind::Local,
                    *mutable,
                    readonly,
                    type_text,
                )?;
                StmtIr::Let { slot, value }
            }
            Stmt::Assign { target, value } => {
                let target = self.lower_assign_target(target)?;
                let value = self.lower_expr(value)?;
                StmtIr::Assign { target, value }
            }
            Stmt::Return(value) => StmtIr::Return {
                value: value
                    .as_ref()
                    .map(|value| self.lower_expr(value))
                    .transpose()?,
            },
            Stmt::Expr(value) => StmtIr::Expr {
                value: self.lower_expr(value)?,
            },
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => StmtIr::If {
                condition: self.lower_expr(condition)?,
                then_block: self.lower_scoped_block("if_then", then_block, |_| Ok(()))?,
                else_block: else_block
                    .as_ref()
                    .map(|block| self.lower_scoped_block("if_else", block, |_| Ok(())))
                    .transpose()?,
            },
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
                let item_readonly = BindingReadonlyFlags {
                    readonly: self.readonly_array_item_for_value(iterable),
                    readonly_array_item: false,
                };
                let item_type = self
                    .next_expression_single_for_item_type()
                    .or_else(|| self.infer_single_for_item_type(iterable));
                let item_type_text = item_type
                    .as_ref()
                    .map(|(source_text, _)| source_text.clone());
                let item_type_ir = item_type.map(|(_, ty)| ty);
                let entry_type_text = self
                    .next_expression_map_entry_type_text()
                    .or_else(|| self.infer_map_entry_type_text(iterable));
                let iterable = self.lower_expr(iterable)?;
                let mut item_slot = None;
                let mut value_slot = None;
                let body = self.lower_scoped_block("for_body", body, |lowerer| {
                    match binding {
                        ForBinding::Item { item } => {
                            item_slot = Some(lowerer.declare_slot_with_type(
                                item,
                                SlotKind::Local,
                                false,
                                item_readonly,
                                item_type_text,
                            )?);
                        }
                        ForBinding::Entry { key, value } => {
                            let (key_type_text, value_type_text) =
                                entry_type_text.clone().ok_or_else(|| {
                                    CompileError::Semantic(
                                        "for entry binding requires Map in File IR unit function"
                                            .to_string(),
                                    )
                                })?;
                            item_slot = Some(lowerer.declare_slot_with_type(
                                key,
                                SlotKind::Local,
                                false,
                                item_readonly,
                                Some(key_type_text),
                            )?);
                            value_slot = Some(lowerer.declare_slot_with_type(
                                value,
                                SlotKind::Local,
                                false,
                                BindingReadonlyFlags::default(),
                                Some(value_type_text),
                            )?);
                        }
                    }
                    Ok(())
                })?;
                StmtIr::ForIn {
                    item_slot: item_slot.expect("for body setup assigns item slot"),
                    item_type: item_type_ir,
                    value_slot,
                    iterable,
                    body,
                }
            }
            Stmt::Match { value, arms } => {
                let value = self.lower_expr(value)?;
                let arms = arms
                    .iter()
                    .map(|arm| self.lower_match_arm(arm))
                    .collect::<Result<Vec<_>>>()?;
                StmtIr::Match { value, arms }
            }
            Stmt::DbTransaction { body } => self.lower_db_transaction_stmt(body)?,
            Stmt::Assert { condition, message } => StmtIr::Assert {
                condition: self.lower_expr(condition)?,
                message: message.as_ref().map(|message| {
                    self.push_expr(ExprIr::Literal {
                        value: LiteralIr::String {
                            value: message.clone(),
                        },
                    })
                }),
            },
            Stmt::Break => StmtIr::Break,
            Stmt::Continue => StmtIr::Continue,
            Stmt::Emit(value) => StmtIr::Emit {
                operation: String::new(),
                value: self.lower_expr(value)?,
            },
            Stmt::Throw { value } => {
                let payload_type = self.throw_payload_type(value, 0)?;
                StmtIr::Throw {
                    value: self.lower_expr(value)?,
                    payload_type,
                }
            }
            Stmt::Rethrow { exception } => StmtIr::Rethrow {
                exception_slot: self.exception_slot(exception)?,
            },
            Stmt::Spawn { call } => self.lower_spawn_stmt(call)?,
        };
        Ok(self.push_stmt(lowered))
    }

    fn lower_spawn_stmt(&mut self, call: &Expr) -> Result<StmtIr> {
        self.reject_spawn_actor_method_target(call)?;
        let call_ref = self.lower_expr(call)?;
        let metadata = self.spawn_function_target_metadata(call_ref)?;
        let Some(ExprIr::Call { call }) =
            self.body.expressions.get_mut(call_ref.expression as usize)
        else {
            return Err(CompileError::Semantic(
                "spawn statement expects a lowered call expression".to_string(),
            ));
        };
        call.metadata
            .insert(SPAWN_SUBMIT_METADATA_KEY.to_string(), metadata);
        Ok(StmtIr::Spawn { call: call_ref })
    }

    fn reject_spawn_actor_method_target(&self, call: &Expr) -> Result<()> {
        let Expr::Call { callee, .. } = call else {
            return Ok(());
        };
        let callee = match callee.as_ref() {
            Expr::Generic { callee, .. } => callee.as_ref(),
            callee => callee,
        };
        let Expr::Field { object, .. } = callee else {
            return Ok(());
        };
        if self
            .infer_receiver_expr_type(object)?
            .is_some_and(|(_, ty)| Self::is_actor_ref_receiver_type(&ty))
        {
            return Err(CompileError::Semantic(
                "spawn actor method calls are no longer supported".to_string(),
            ));
        }
        Ok(())
    }

    fn spawn_function_target_metadata(&self, call_ref: ExprRefIr) -> Result<MetadataValue> {
        let Some(ExprIr::Call { call }) = self.body.expressions.get(call_ref.expression as usize)
        else {
            return Err(CompileError::Semantic(
                "spawn statement expects a call expression".to_string(),
            ));
        };
        let (target_kind, target) = match &call.target {
            CallTargetIr::LocalExecutable { executable_index } => {
                let declaration_name = self
                    .executable_indices
                    .iter()
                    .find_map(|(name, index)| {
                        (*index == *executable_index).then_some(name.as_str())
                    })
                    .ok_or_else(|| {
                        CompileError::Semantic(format!(
                            "spawn target executable index {executable_index} is not declared in module {}",
                            self.module_path
                        ))
                    })?;
                if declaration_name.contains('.') {
                    return Err(CompileError::Semantic(
                        "spawn supports function calls; ordinary impl method calls cannot be spawned"
                            .to_string(),
                    ));
                }
                (
                    "function",
                    format!(
                        "{SPAWN_FUNCTION_TARGET_PREFIX}{}",
                        executable_symbol(self.module_path, declaration_name)
                    ),
                )
            }
            CallTargetIr::ExternalServiceSymbol { symbol } => (
                "function",
                format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", symbol.symbol_path()),
            ),
            CallTargetIr::PackageSymbol {
                package_ref,
                operation,
            } => {
                let mut metadata = BTreeMap::new();
                metadata.insert(
                    "targetKind".to_string(),
                    MetadataValue::String("function".to_string()),
                );
                metadata.insert(
                    "target".to_string(),
                    MetadataValue::String(format!("package:{}", operation.public_path)),
                );
                match package_ref {
                    PackageRefIr::Dependency { dependency_ref } => {
                        metadata.insert(
                            "packageDependencyRef".to_string(),
                            MetadataValue::String(dependency_ref.clone()),
                        );
                    }
                    PackageRefIr::PackageId { package_id } => {
                        metadata.insert(
                            "packageId".to_string(),
                            MetadataValue::String(package_id.clone()),
                        );
                    }
                }
                metadata.insert(
                    "packageOperationAbiId".to_string(),
                    MetadataValue::String(operation.operation_abi_id.clone()),
                );
                metadata.insert(
                    "packageOperationPath".to_string(),
                    MetadataValue::String(operation.public_path.clone()),
                );
                return Ok(MetadataValue::Object(metadata));
            }
            _ => {
                return Err(CompileError::Semantic(
                    "spawn currently supports only function calls".to_string(),
                ));
            }
        };
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "targetKind".to_string(),
            MetadataValue::String(target_kind.to_string()),
        );
        metadata.insert("target".to_string(), MetadataValue::String(target));
        Ok(MetadataValue::Object(metadata))
    }

    fn readonly_flags_for_value(&self, value: &Expr) -> BindingReadonlyFlags {
        self.readonly_flags_for_value_with_locals(value, &BTreeMap::new())
    }

    fn readonly_array_item_for_value(&self, value: &Expr) -> bool {
        self.readonly_flags_for_value(value).readonly_array_item
    }

    fn readonly_flags_for_value_with_locals(
        &self,
        value: &Expr,
        locals: &BTreeMap<String, BindingReadonlyFlags>,
    ) -> BindingReadonlyFlags {
        match value {
            Expr::DbOperation(operation) if is_db_readonly_result_operation(operation) => {
                BindingReadonlyFlags {
                    readonly: true,
                    readonly_array_item: operation.op == DbOperationKind::Find && operation.many,
                }
            }
            Expr::DbQuery(_) => BindingReadonlyFlags {
                readonly: true,
                readonly_array_item: false,
            },
            Expr::DbLeaseRead(_) => BindingReadonlyFlags {
                readonly: true,
                readonly_array_item: false,
            },
            Expr::DbTransaction(transaction) => {
                self.readonly_flags_for_transaction_value(transaction)
            }
            Expr::Identifier(name) => locals.get(name).copied().unwrap_or_else(|| {
                self.bindings
                    .get(name)
                    .map(|binding| BindingReadonlyFlags {
                        readonly: binding.readonly,
                        readonly_array_item: binding.readonly_array_item,
                    })
                    .unwrap_or_default()
            }),
            Expr::Field { object, .. } => {
                let mut flags = self.readonly_flags_for_value_with_locals(object, locals);
                flags.readonly_array_item = false;
                flags
            }
            _ => BindingReadonlyFlags::default(),
        }
    }

    fn readonly_flags_for_transaction_value(
        &self,
        transaction: &skiff_syntax::ast::DbTransaction,
    ) -> BindingReadonlyFlags {
        if transaction.mode != DbBlockMode::Value {
            return BindingReadonlyFlags::default();
        }
        let Some((Stmt::Expr(value), prefix)) = transaction.body.statements.split_last() else {
            return BindingReadonlyFlags::default();
        };
        let mut locals = BTreeMap::new();
        for stmt in prefix {
            if let Stmt::Let { name, value, .. } = stmt {
                let flags = self.readonly_flags_for_value_with_locals(value, &locals);
                locals.insert(name.clone(), flags);
            }
        }
        self.readonly_flags_for_value_with_locals(value, &locals)
    }

    fn lower_assign_target(&mut self, target: &Expr) -> Result<AssignTargetIr> {
        match target {
            Expr::Identifier(name) => {
                self.next_expression_key();
                let Some(binding) = self.bindings.get(name) else {
                    return Err(CompileError::Semantic(format!(
                        "unresolved assignment target `{name}` in File IR unit function"
                    )));
                };
                if !binding.mutable {
                    return Err(CompileError::Semantic(format!(
                        "cannot assign to immutable binding `{name}` in File IR unit function"
                    )));
                }
                Ok(AssignTargetIr::Slot { slot: binding.slot })
            }
            Expr::Field { object, field } => {
                self.next_expression_key();
                Ok(AssignTargetIr::Field {
                    object: {
                        if let Some(name) = self.readonly_assignment_base_identifier(object) {
                            return Err(CompileError::Semantic(format!(
                            "cannot assign to field of readonly binding `{name}` in File IR unit function"
                        )));
                        }
                        self.lower_expr(object)?
                    },
                    field: field.clone(),
                })
            }
            _ => Err(unsupported(
                "only slot and field assignment targets are supported by the File IR unit emitter",
            )),
        }
    }

    fn readonly_assignment_base_identifier<'b>(&self, target: &'b Expr) -> Option<&'b str> {
        match target {
            Expr::Identifier(name)
                if self
                    .bindings
                    .get(name)
                    .map(|binding| binding.readonly)
                    .unwrap_or(false) =>
            {
                Some(name.as_str())
            }
            Expr::Field { object, .. } => self.readonly_assignment_base_identifier(object),
            _ => None,
        }
    }

    pub(super) fn consume_expression_key(&mut self) {
        self.next_expression_key();
    }

    fn next_expression_key(&mut self) -> Option<ExpressionKey> {
        let key = self.peek_expression_key();
        self.next_expression_index += 1;
        key
    }

    fn peek_expression_key(&self) -> Option<ExpressionKey> {
        self.expression_owner.as_ref().map(|owner| {
            ExpressionKey::new(
                self.module_path.to_string(),
                owner.clone(),
                self.next_expression_index,
            )
        })
    }

    fn type_resolution_context(&self) -> TypeResolutionContext<'_> {
        TypeResolutionContext::with_type_params(self.module_path, self.type_param_scope.clone())
    }

    fn required_expression_type_fact(
        &self,
        key: &ExpressionKey,
        purpose: &str,
    ) -> Result<(String, TypeRefIr)> {
        let expression_types = self.expression_types.ok_or_else(|| {
            CompileError::Semantic(format!(
                "{purpose} lowering requires expression type facts; missing model while looking up ExpressionKey {:?}",
                key
            ))
        })?;
        let fact = expression_types.fact(key).ok_or_else(|| {
            CompileError::Semantic(format!(
                "{purpose} lowering requires expression type fact for ExpressionKey {:?}",
                key
            ))
        })?;
        let ty = fact.ty.as_ref().ok_or_else(|| {
            CompileError::Semantic(format!(
                "{purpose} lowering requires concrete expression type for ExpressionKey {:?}",
                key
            ))
        })?;
        Ok((ty.source_text.clone(), ty.ir.clone()))
    }

    fn lower_record_construct(
        &mut self,
        key: Option<&ExpressionKey>,
        type_name: &str,
        type_args: &[TypeRef],
        fields: &[(String, Expr)],
    ) -> Result<ExprIr> {
        let validation = match (self.expression_types, key) {
            (Some(expression_types), Some(key)) => Some(
                expression_types
                    .constructor_validation(key)
                    .cloned()
                    .ok_or_else(|| {
                        CompileError::Semantic(format!(
                            "missing constructor validation fact for expression {:?} constructing `{type_name}` with fields [{}]",
                            key,
                            fields
                                .iter()
                                .map(|(field, _)| field.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    })?,
            ),
            (Some(_), None) => {
                return Err(CompileError::Semantic(
                    "missing expression owner while lowering typed record construct".to_string(),
                ));
            }
            (None, _) => None,
        };

        let mut provided_values = BTreeMap::new();
        for (field, value) in fields {
            if validation.is_none() && provided_values.contains_key(field) {
                return Err(CompileError::Semantic(format!(
                    "duplicate record construct field `{field}` in File IR unit expression"
                )));
            }
            let value_key = self.peek_expression_key();
            let value = self.lower_expr(value)?;
            provided_values.insert(field.clone(), (value_key, value));
        }

        let Some(validation) = validation else {
            return Ok(ExprIr::Construct {
                type_ref: lower_named_type(
                    type_name,
                    type_args,
                    self.type_indices,
                    self.local_db_objects,
                    self.publication_db_metadata,
                    self.package_aliases,
                    self.external_type_symbols,
                    self.source_alias_targets,
                    self.value_type_context(),
                )?,
                fields: provided_values
                    .into_iter()
                    .map(|(field, (_, value))| (field, value))
                    .collect(),
            });
        };

        let mut lowered_fields = BTreeMap::new();
        for field in validation.materialized_fields {
            let value = match field.source {
                ConstructorFieldValueSource::Provided {
                    field_name,
                    expression,
                } => {
                    let Some((actual_key, value)) = provided_values.get(&field_name) else {
                        return Err(CompileError::Semantic(format!(
                            "constructor validation for `{type_name}` references missing provided field `{field_name}`"
                        )));
                    };
                    if actual_key
                        .as_ref()
                        .is_some_and(|actual_key| actual_key != &expression)
                    {
                        return Err(CompileError::Semantic(format!(
                            "constructor validation for `{type_name}` field `{field_name}` points to expression {:?}, but lowering reached {:?}",
                            expression,
                            actual_key
                        )));
                    }
                    value.clone()
                }
                ConstructorFieldValueSource::SyntheticNull => self.push_expr(ExprIr::Literal {
                    value: LiteralIr::Null,
                }),
            };
            lowered_fields.insert(field.name, value);
        }

        Ok(ExprIr::Construct {
            type_ref: validation.target.ir,
            fields: lowered_fields,
        })
    }

    fn lower_interface_box(
        &mut self,
        expression_key: Option<&ExpressionKey>,
        value: &Expr,
        interface: &TypeRef,
    ) -> Result<ExprIr> {
        let box_key = expression_key.ok_or_else(|| {
            CompileError::Semantic(
                "interface boxing lowering requires an ExpressionKey for the box expression"
                    .to_string(),
            )
        })?;
        let context = self.type_resolution_context();
        let selector = self
            .type_resolution
            .resolve_canonical_interface_selector_type_ref(interface, &context)
            .map_err(|error| {
                CompileError::Semantic(format!(
                    "interface boxing selector `{}` at ExpressionKey {:?} is missing or invalid canonical selector fact: {error}",
                    interface.name, box_key
                ))
            })?;

        let (_, box_type) =
            self.required_expression_type_fact(box_key, "interface boxing expression")?;
        match box_type {
            TypeRefIr::AnyInterface { interface } if interface == selector.instantiation_ref => {}
            TypeRefIr::AnyInterface { interface } => {
                return Err(CompileError::Semantic(format!(
                    "interface boxing expression fact at ExpressionKey {:?} resolved selector {:?}, but AST selector `{}` resolved to {:?}",
                    box_key, interface, selector.source_text, selector.instantiation_ref
                )));
            }
            other => {
                return Err(CompileError::Semantic(format!(
                    "interface boxing expression fact at ExpressionKey {:?} must be AnyInterface for selector `{}`, found {}",
                    box_key,
                    selector.source_text,
                    type_ref_ir_type_text(&other)
                )));
            }
        }

        if let Expr::RemotePublicInstanceSource(source) = value {
            let expected_value_key =
                expression_key_offset(box_key, 1, "remote interface box source")?;
            let actual_value_key = self.next_expression_key().ok_or_else(|| {
                CompileError::Semantic(
                    "remote interface boxing lowering requires an ExpressionKey for the source expression"
                        .to_string(),
                )
            })?;
            if actual_value_key != expected_value_key {
                return Err(CompileError::Semantic(format!(
                    "remote interface boxing source expected ExpressionKey {:?}, but lowering reached {:?}",
                    expected_value_key, actual_value_key
                )));
            }
            let projection = self
                .expression_types
                .and_then(|expression_types| expression_types.remote_interface_box(box_key))
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "missing remote interface boxing resolution fact for ExpressionKey {:?}",
                        box_key
                    ))
                })?;
            if projection.dependency_ref != source.dependency_ref
                || projection.public_instance_key != source.public_instance_key
                || projection.interface != selector.instantiation_ref
            {
                return Err(CompileError::Semantic(format!(
                    "remote interface boxing resolution for ExpressionKey {:?} does not match source `{}/{}` as `{}`",
                    box_key, source.dependency_ref, source.public_instance_key, selector.source_text
                )));
            }
            let value = self.push_expr(ExprIr::Literal {
                value: LiteralIr::Null,
            });
            return Ok(ExprIr::InterfaceBox {
                value,
                interface: selector.instantiation_ref,
                source: BoxSourceIr::Remote {
                    dependency_ref: source.dependency_ref.clone(),
                    public_instance_key: source.public_instance_key.clone(),
                    operations: RemoteOperationTablePlanIr {
                        interface: projection.interface.clone(),
                        slots: projection
                            .slots
                            .iter()
                            .map(|slot| RemoteOperationSlotPlanIr {
                                slot: slot.slot,
                                method_abi_id: slot.method_abi_id.clone(),
                                signature: InterfaceMethodSlotSignatureIr {
                                    params: slot.params.clone(),
                                    return_type: slot.return_type.clone(),
                                },
                                operation_abi_id: slot.operation.operation_abi_id.clone(),
                            })
                            .collect(),
                    },
                    callee_protocol_identity: projection.callee_protocol_identity.clone(),
                },
            });
        }

        let value_key = expression_key_offset(box_key, 1, "interface boxing value")?;
        let (value_source_text, concrete_type) =
            self.required_expression_type_fact(&value_key, "interface boxing value")?;
        let actual = ResolvedTypeRef {
            source_text: value_source_text.clone(),
            ir: concrete_type.clone(),
        };
        if self
            .type_resolution
            .concrete_nominal_record_symbol(&actual, &context)
            .is_none()
        {
            return Err(CompileError::Semantic(format!(
                "interface boxing value at ExpressionKey {:?} must be a concrete nominal record, found {}",
                value_key, value_source_text
            )));
        }
        let expected = ResolvedTypeRef {
            source_text: selector.source_text.clone(),
            ir: selector.identity.clone(),
        };
        let conformance = self
            .type_resolution
            .local_any_interface_conformance_for_boxing(&actual, &expected, &context)
            .map_err(|error| {
                CompileError::Semantic(format!(
                    "interface boxing local conformance lookup for selector `{}` at ExpressionKey {:?} failed: {error}",
                    selector.source_text, box_key
                ))
            })?
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "interface boxing value at ExpressionKey {:?} type {} does not explicitly implement selector `{}`",
                    value_key, value_source_text, selector.source_text
                ))
            })?;
        if conformance.interface != selector.instantiation_ref {
            return Err(CompileError::Semantic(format!(
                "interface boxing conformance for ExpressionKey {:?} resolved interface {:?}, but selector `{}` resolved to {:?}",
                box_key, conformance.interface, selector.source_text, selector.instantiation_ref
            )));
        }
        let method_table = self.lower_local_interface_method_table(&conformance, &concrete_type)?;
        let value = self.lower_expr(value)?;
        Ok(ExprIr::InterfaceBox {
            value,
            interface: selector.instantiation_ref,
            source: BoxSourceIr::Local {
                concrete_type,
                method_table,
            },
        })
    }

    fn lower_local_interface_method_table(
        &self,
        conformance: &skiff_compiler_source::LocalAnyInterfaceConformanceResolution,
        concrete_type: &TypeRefIr,
    ) -> Result<InterfaceMethodTablePlanIr> {
        let mut slots = Vec::with_capacity(conformance.slots.len());
        for slot in &conformance.slots {
            let executable_index = self
                .local_impl_method_executable_index(conformance.receiver.symbol(), &slot.name)
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "interface method table slot {} `{}` for receiver {} has no local impl method executable",
                        slot.slot, slot.name, conformance.receiver
                    ))
                })?;
            let signature = self
                .executable_signatures
                .get(&executable_index)
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "interface method table slot {} `{}` for receiver {} targets executable index {} without a lowered signature",
                        slot.slot, slot.name, conformance.receiver, executable_index
                    ))
                })?;
            slots.push(InterfaceMethodSlotPlanIr {
                slot: slot.slot,
                method_name: slot.name.clone(),
                method_abi_id: slot.method_abi_id.clone(),
                signature: InterfaceMethodSlotSignatureIr {
                    params: interface_method_slot_signature_params(signature, concrete_type),
                    return_type: signature.return_type.clone(),
                },
                target: InterfaceMethodSlotTargetIr {
                    executable_index,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
            });
        }
        Ok(InterfaceMethodTablePlanIr {
            interface: conformance.interface.clone(),
            concrete_type: concrete_type.clone(),
            slots,
        })
    }

    fn consume_static_callee_expression_keys(&mut self, callee: &Expr) -> Result<()> {
        match callee {
            Expr::Identifier(_) => {
                self.next_expression_key();
                Ok(())
            }
            Expr::Field { object, .. } => {
                self.next_expression_key();
                self.consume_static_callee_expression_keys(object)
            }
            Expr::RemotePublicInstanceSource(_) => {
                self.next_expression_key();
                Ok(())
            }
            Expr::Generic { callee, .. } => {
                self.next_expression_key();
                self.consume_static_callee_expression_keys(callee)
            }
            _ => Err(unsupported_call(callee)),
        }
    }

    pub(super) fn lower_expr(&mut self, expr: &Expr) -> Result<ExprRefIr> {
        let expression_key = self.next_expression_key();
        let lowered = match expr {
            Expr::Literal(literal) => ExprIr::Literal {
                value: lower_literal(literal)?,
            },
            Expr::Identifier(name) => {
                if let Some(binding) = self.bindings.get(name) {
                    ExprIr::LoadSlot { slot: binding.slot }
                } else if let Some(const_index) = self.const_indices.get(name) {
                    ExprIr::LoadConst {
                        const_index: *const_index,
                    }
                } else {
                    return Err(CompileError::Semantic(format!(
                        "unresolved identifier `{name}` in File IR unit expression"
                    )));
                }
            }
            Expr::Unary { op, expr } => ExprIr::Unary {
                op: lower_unary_op(*op),
                value: self.lower_expr(expr)?,
            },
            Expr::Binary { op, left, right } => ExprIr::Binary {
                op: lower_binary_op(*op),
                left: self.lower_expr(left)?,
                right: self.lower_expr(right)?,
            },
            Expr::ObjectLiteral { entries } => {
                let mut lowered = BTreeMap::new();
                for entry in entries {
                    let key = self.lower_object_literal_key(&entry.key)?;
                    if lowered.contains_key(&key) {
                        return Err(CompileError::Semantic(format!(
                            "duplicate object literal key `{key}` in File IR unit expression"
                        )));
                    }
                    let value = self.lower_expr(&entry.value)?;
                    lowered.insert(key, value);
                }
                ExprIr::MapLiteral { entries: lowered }
            }
            Expr::Patch { target, operations } => self.lower_patch_expr(target, operations)?,
            Expr::Record {
                type_name,
                type_args,
                fields,
            } => self.lower_record_construct(expression_key.as_ref(), type_name, type_args, fields)?,
            Expr::Field { object, field } => ExprIr::Field {
                object: self.lower_expr(object)?,
                field: field.clone(),
            },
            Expr::Call { callee, args } => {
                if let Some(payload) =
                    self.lower_representation_constructor_call(expression_key.as_ref(), callee, args)?
                {
                    return Ok(payload);
                }
                self.lower_call(callee, args)?
            }
            Expr::Generic { .. } => {
                return Err(unsupported(
                    "generic expressions are only supported as part of record constructs in the File IR unit emitter",
                ))
            }
            Expr::InterfaceBox { value, interface } => {
                self.lower_interface_box(expression_key.as_ref(), value, interface)?
            }
            Expr::RemotePublicInstanceSource(source) => {
                return Err(CompileError::Semantic(format!(
                    "remote public instance source `{}/{}` is not a value; use `as I` or call a method directly",
                    source.dependency_ref, source.public_instance_key
                )));
            }
            Expr::Throw { value } => {
                let payload_type = self.throw_payload_type(value, 1)?;
                ExprIr::Throw {
                    value: self.lower_expr(value)?,
                    payload_type,
                }
            }
            Expr::Rethrow { exception } => ExprIr::Rethrow {
                exception_slot: self.exception_slot(exception)?,
            },
            Expr::Catch {
                catch_type,
                try_expr,
            } => {
                let catch_name = format!("$catch{}", self.slots.len());
                let catch_slot = self.declare_slot(&catch_name, SlotKind::Temp, false)?;
                ExprIr::Catch {
                    try_expression: self.lower_expr(try_expr)?,
                    catch_slot,
                    catch_type: Some(lower_type_ref(
                        catch_type,
                        self.type_indices,
                        self.local_db_objects,
                        self.publication_db_metadata,
                        self.package_aliases,
                        self.external_type_symbols,
                        self.source_alias_targets,
                        self.value_type_context(),
                    )?),
                    body: self.push_expr(ExprIr::LoadSlot { slot: catch_slot }),
                }
            }
            Expr::DbOperation(operation) => self.lower_db_operation(operation)?,
            Expr::DbQuery(query) => self.lower_db_query_value(query)?,
            Expr::DbTransaction(transaction) => self.lower_db_transaction_expr(transaction)?,
            Expr::DbLeaseClaim(claim) => self.lower_db_lease_claim(claim)?,
            Expr::DbLeaseRead(read) => self.lower_db_lease_read(read)?,
        };
        Ok(self.push_expr(lowered))
    }

    fn lower_patch_expr(
        &mut self,
        target: &TypeRef,
        operations: &[PatchOperation],
    ) -> Result<ExprIr> {
        let mut set_entries = BTreeMap::new();
        let mut inc_entries = BTreeMap::new();
        for operation in operations {
            let (kind, path, value) = match operation {
                PatchOperation::Set { path, value } => ("set", path, value),
                PatchOperation::Inc { path, value } => ("inc", path, value),
            };
            let key = path.join(".");
            let entries = match kind {
                "set" => &mut set_entries,
                "inc" => &mut inc_entries,
                _ => unreachable!("typed patch operation kind is exhaustive"),
            };
            if entries.contains_key(&key) {
                return Err(CompileError::Semantic(format!(
                    "duplicate patch {kind} field `{key}` in File IR unit expression"
                )));
            }
            let lowered_value = self.lower_expr(value)?;
            entries.insert(key, lowered_value);
        }

        let patch_type = self.push_expr(ExprIr::Literal {
            value: LiteralIr::String {
                value: target.name.clone(),
            },
        });
        let set = self.push_expr(ExprIr::MapLiteral {
            entries: set_entries,
        });
        let inc = self.push_expr(ExprIr::MapLiteral {
            entries: inc_entries,
        });

        let mut entries = BTreeMap::new();
        entries.insert("__skiffPatchType".to_string(), patch_type);
        entries.insert("set".to_string(), set);
        entries.insert("inc".to_string(), inc);
        Ok(ExprIr::MapLiteral { entries })
    }

    fn lower_call(&mut self, callee: &Expr, args: &[Expr]) -> Result<ExprIr> {
        let (callee, type_arg_refs) = match callee {
            Expr::Generic { callee, type_args } => {
                self.next_expression_key();
                (callee.as_ref(), type_args.as_slice())
            }
            _ => (callee, &[][..]),
        };
        let mut lowered_args = Vec::new();
        let target = if let Expr::Field { object, field } = callee {
            if let Some(target) = self.remote_public_instance_direct_call_target(object, field)? {
                self.consume_static_callee_expression_keys(callee)?;
                target
            } else if let Some(target) = self.lower_receiver_call_target(object, field)? {
                self.next_expression_key();
                lowered_args.push(self.lower_expr(object)?);
                target
            } else {
                self.consume_static_callee_expression_keys(callee)?;
                self.lower_static_call_target(callee)?
            }
        } else {
            self.consume_static_callee_expression_keys(callee)?;
            self.lower_static_call_target(callee)?
        };
        let mut type_args = type_arg_refs
            .iter()
            .enumerate()
            .map(|(index, ty)| {
                Ok((
                    format!("T{index}"),
                    lower_type_ref(
                        ty,
                        self.type_indices,
                        self.local_db_objects,
                        self.publication_db_metadata,
                        self.package_aliases,
                        self.external_type_symbols,
                        self.source_alias_targets,
                        self.value_type_context(),
                    )?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        self.infer_native_call_type_args(&target, &mut type_args, args);
        let metadata = match &target {
            CallTargetIr::Builtin { op } if is_db_builtin_op(op) => self.lower_db_call_metadata(
                op,
                type_arg_refs,
                type_args.keys().next().map(String::as_str),
                args,
            )?,
            _ => BTreeMap::new(),
        };
        for arg in args {
            lowered_args.push(self.lower_expr(arg)?);
        }
        Ok(ExprIr::Call {
            call: CallIr {
                target,
                args: lowered_args,
                type_args,
                metadata,
            },
        })
    }

    fn infer_native_call_type_args(
        &self,
        target: &CallTargetIr,
        type_args: &mut BTreeMap<String, TypeRefIr>,
        args: &[Expr],
    ) {
        if type_args.contains_key("T0") {
            return;
        }
        let CallTargetIr::Native { target } = target else {
            return;
        };
        if !is_std_http_json_native_target(target) {
            return;
        }
        let Some(payload_type) = self.native_http_json_payload_type(args) else {
            return;
        };
        type_args.insert("T0".to_string(), payload_type);
    }

    fn native_http_json_payload_type(&self, args: &[Expr]) -> Option<TypeRefIr> {
        let payload_index = 1;
        let payload = args.get(payload_index)?;
        self.call_argument_type_at_index(args, payload_index)
            .or_else(|| self.infer_expr_type_ir(payload))
    }

    fn call_argument_type_at_index(&self, args: &[Expr], index: usize) -> Option<TypeRefIr> {
        let offset = args.iter().take(index).map(expr_preorder_node_count).sum();
        self.expression_type_at_offset(offset).map(|(_, ty)| ty)
    }

    fn lower_representation_constructor_call(
        &mut self,
        expression_key: Option<&ExpressionKey>,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<Option<ExprRefIr>> {
        let representation_validation = expression_key
            .and_then(|key| {
                self.expression_types.and_then(|expression_types| {
                    expression_types.representation_constructor_validation(key)
                })
            })
            .cloned();
        if let Some(validation) = representation_validation {
            let payload_expression = validation.payload;
            let _erased_wrapper_type = validation.target;
            let callee = match callee {
                Expr::Generic { callee, .. } => {
                    self.next_expression_key();
                    callee.as_ref()
                }
                _ => callee,
            };
            self.consume_static_callee_expression_keys(callee)?;
            let [payload] = args else {
                return Err(CompileError::Semantic(
                    "representation constructor lowering expected exactly one payload argument"
                        .to_string(),
                ));
            };
            let payload_key = self.peek_expression_key();
            if payload_key
                .as_ref()
                .is_some_and(|payload_key| payload_key != &payload_expression)
            {
                return Err(CompileError::Semantic(format!(
                    "representation constructor validation points to expression {:?}, but lowering reached {:?}",
                    payload_expression,
                    payload_key
                )));
            }
            let payload = self.lower_expr(payload)?;
            return Ok(Some(payload));
        }
        Ok(None)
    }

    fn lower_receiver_call_target(
        &self,
        object: &Expr,
        method_name: &str,
    ) -> Result<Option<CallTargetIr>> {
        if !self.is_receiver_call(object) {
            return Ok(None);
        }

        let Some((receiver_text, receiver_ty)) = self.receiver_type_for_call_object(object)? else {
            return Err(CompileError::Semantic(format!(
                "receiver method `{method_name}` requires a statically known receiver type"
            )));
        };

        if let Some(target) =
            self.lower_any_interface_receiver_call_target(&receiver_ty, method_name)
        {
            return Ok(Some(target));
        }

        if let Some(op) = Self::builtin_receiver_op_for_type(&receiver_ty, method_name) {
            return Ok(Some(CallTargetIr::ReceiverBuiltin { op }));
        }

        if Self::is_actor_ref_receiver_type(&receiver_ty) {
            return Err(CompileError::Semantic(format!(
                "ActorRef receiver method calls are no longer supported: `{method_name}`; spawn a function instead"
            )));
        }

        if self.is_interface_receiver_method(&receiver_ty, method_name) {
            return Err(CompileError::Semantic(format!(
                "interface receiver method `{method_name}` requires a controlled receiver root; ordinary interface values are not supported"
            )));
        }

        if self
            .package_interface_methods
            .is_interface_method(&receiver_ty, method_name)
        {
            return Err(CompileError::Semantic(format!(
                "package interface receiver method `{method_name}` requires a controlled receiver root; ordinary interface values are not supported"
            )));
        }

        if self
            .package_interface_methods
            .is_interface_type(&receiver_ty)
        {
            return Err(CompileError::Semantic(format!(
                "package interface receiver method `{method_name}` is not available on ordinary interface values; use a controlled receiver root"
            )));
        }

        if let Some(target) = self.lower_static_impl_receiver_call_target(&receiver_ty, method_name)
        {
            return Ok(Some(target));
        }

        Err(CompileError::Semantic(format!(
            "receiver method `{method_name}` on `{receiver_text}` must resolve to a local/package executable, receiver builtin op, interface receiver root, or ActorRef"
        )))
    }

    fn lower_any_interface_receiver_call_target(
        &self,
        receiver_ty: &TypeRefIr,
        method_name: &str,
    ) -> Option<CallTargetIr> {
        let resolution = self
            .type_resolution
            .any_interface_method_signature(receiver_ty, method_name)?;
        Some(CallTargetIr::InterfaceMethod {
            interface: resolution.interface,
            method_abi_id: resolution.method_abi_id,
            slot: resolution.slot,
        })
    }

    fn receiver_type_for_call_object(&self, object: &Expr) -> Result<Option<(String, TypeRefIr)>> {
        if let Some((source_text, ty)) = self.expression_type_at_offset(1) {
            let ty = match ty {
                TypeRefIr::AnyInterface { .. } => ty,
                _ => self.lower_receiver_type_text(&source_text).unwrap_or(ty),
            };
            if !is_unknown_type_ref(&ty) {
                return Ok(Some((source_text, ty)));
            }
        }
        self.infer_receiver_expr_type(object)
    }

    fn lower_receiver_type_text(&self, source_text: &str) -> Option<TypeRefIr> {
        lower_type_text(
            source_text,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.value_type_context(),
        )
        .ok()
    }

    fn infer_receiver_expr_type(&self, expr: &Expr) -> Result<Option<(String, TypeRefIr)>> {
        match expr {
            Expr::Field { object, field } => {
                let Some((_, object_ty)) = self.infer_receiver_expr_type(object)? else {
                    return Ok(None);
                };
                Ok(self
                    .field_type_for_receiver_type(&object_ty, field)
                    .map(|ty| (type_ref_ir_type_text(&ty), ty)))
            }
            _ => Ok(self
                .infer_expr_type_ir(expr)
                .map(|ty| (type_ref_ir_type_text(&ty), ty))),
        }
    }

    pub(super) fn field_type_for_receiver_type(
        &self,
        ty: &TypeRefIr,
        field: &str,
    ) -> Option<TypeRefIr> {
        let direct = match ty {
            TypeRefIr::Native { name, args } => self.builtin_field_type(name, args, field),
            TypeRefIr::LocalType { type_index } => self
                .local_type_fields
                .get(type_index)
                .and_then(|fields| fields.get(field))
                .cloned(),
            TypeRefIr::Record { fields } => fields.get(field).cloned(),
            TypeRefIr::PackageSymbol { symbol } => self.std_package_field_type(symbol, field),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => self
                .db_metadata_field_type(
                    &format!("{}.{}", symbol.module_path, symbol.symbol),
                    field,
                ),
            TypeRefIr::Union { items } => {
                let mut field_types = Vec::new();
                for item in items {
                    field_types.push(self.field_type_for_receiver_type(item, field)?);
                }
                Some(union_type_ir(field_types))
            }
            TypeRefIr::Nullable { inner } => self.field_type_for_receiver_type(inner, field),
            _ => None,
        };
        direct.or_else(|| self.resolved_field_type_for_receiver_type(ty, field))
    }

    fn resolved_field_type_for_receiver_type(
        &self,
        ty: &TypeRefIr,
        field: &str,
    ) -> Option<TypeRefIr> {
        let context = TypeResolutionContext::with_type_params(
            self.module_path,
            self.type_param_scope.clone(),
        );
        self.type_resolution
            .record_field_type(
                &ResolvedTypeRef {
                    ir: ty.clone(),
                    source_text: type_ref_ir_type_text(ty),
                },
                field,
                &context,
            )
            .map(|resolved| resolved.ir)
    }

    fn db_metadata_field_type(&self, type_path: &str, field: &str) -> Option<TypeRefIr> {
        self.lowered_publication_db_metadata
            .resolve_qualified(type_path)
            .and_then(|metadata| metadata.field_types.get(field).cloned())
    }

    fn builtin_field_type(&self, name: &str, args: &[TypeRefIr], field: &str) -> Option<TypeRefIr> {
        if name == "DbUpsertResult" {
            return match field {
                "inserted" => Some(TypeRefIr::native("bool")),
                "value" => args.first().cloned(),
                _ => None,
            };
        }
        self.prelude_builtin_field_type(name, field)
    }

    fn std_package_field_type(&self, symbol: &PackageSymbolRef, field: &str) -> Option<TypeRefIr> {
        if let Some(field_ty) = self.package_db_metadata_field_type(symbol, field) {
            return Some(field_ty);
        }
        if !is_std_package_ref(&symbol.package) {
            return None;
        }
        let registry = prelude_registry();
        let canonical_symbol = registry
            .known_type_symbol(&symbol.symbol_path)
            .or_else(|| registry.known_type_symbol(&format!("std.{}", symbol.symbol_path)))?;
        let (module_path, decl_name) = canonical_symbol.rsplit_once('.')?;
        let decl = registry.type_decl(decl_name)?;
        let field = decl
            .fields
            .iter()
            .find(|candidate| candidate.name == field)?;
        let field_type = prelude_field_type_text(&field.ty.name, module_path);
        lower_type_text(
            &field_type,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.value_type_context(),
        )
        .ok()
    }

    fn package_db_metadata_field_type(
        &self,
        symbol: &PackageSymbolRef,
        field: &str,
    ) -> Option<TypeRefIr> {
        let PackageRefIr::Dependency { dependency_ref } = &symbol.package else {
            return None;
        };
        self.db_metadata_field_type(&format!("{dependency_ref}.{}", symbol.symbol_path), field)
    }

    fn prelude_builtin_field_type(&self, name: &str, field: &str) -> Option<TypeRefIr> {
        let registry = prelude_registry();
        let decl_name = registry.prelude_type_decl_name(name)?;
        let decl = registry.type_decl(decl_name)?;
        let module_path = registry.type_decl_module(decl_name)?;
        let field = decl
            .fields
            .iter()
            .find(|candidate| candidate.name == field)?;
        let field_type = prelude_field_type_text(&field.ty.name, module_path);
        lower_type_text(
            &field_type,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.value_type_context(),
        )
        .ok()
    }

    fn lower_static_impl_receiver_call_target(
        &self,
        receiver_ty: &TypeRefIr,
        method_name: &str,
    ) -> Option<CallTargetIr> {
        match receiver_ty {
            TypeRefIr::LocalType { .. } => {
                let type_name = self.local_type_name_for_receiver_type(receiver_ty)?;
                let executable_index =
                    self.local_impl_method_executable_index(type_name, method_name)?;
                Some(CallTargetIr::LocalExecutable { executable_index })
            }
            TypeRefIr::ServiceSymbol { symbol } => Some(CallTargetIr::ExternalServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: symbol.module_path.clone(),
                    symbol: impl_method_declaration_name(&symbol.symbol, method_name),
                },
            }),
            TypeRefIr::PackageSymbol { .. } => None,
            _ => None,
        }
    }

    fn is_interface_receiver_method(&self, receiver_ty: &TypeRefIr, method_name: &str) -> bool {
        let Some(symbol) = self.interface_receiver_symbol(receiver_ty) else {
            return false;
        };
        self.interface_semantics
            .interface(&symbol)
            .is_some_and(|interface| {
                interface
                    .requirements
                    .iter()
                    .any(|requirement| requirement.name == method_name)
            })
    }

    fn interface_receiver_symbol(&self, receiver_ty: &TypeRefIr) -> Option<SourceSymbolKey> {
        match receiver_ty {
            TypeRefIr::LocalType { .. } => {
                let type_name = self.local_type_name_for_receiver_type(receiver_ty)?;
                Some(SourceSymbolKey::new(self.module_path, type_name))
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                Some(SourceSymbolKey::new(module_path, &symbol.symbol))
            }
            TypeRefIr::Nullable { inner } => self.interface_receiver_symbol(inner),
            _ => None,
        }
    }

    fn local_impl_method_executable_index(
        &self,
        type_name: &str,
        method_name: &str,
    ) -> Option<u32> {
        let declaration_name = impl_method_declaration_name(type_name, method_name);
        if let Some(executable_index) = self.executable_indices.get(&declaration_name).copied() {
            return Some(executable_index);
        }
        self.executable_indices
            .iter()
            .find_map(|(declaration_name, executable_index)| {
                let (target, method) = declaration_name.rsplit_once('.')?;
                if method != method_name {
                    return None;
                }
                let target_root = generic_parts(target)
                    .map(|parts| parts.root.trim())
                    .unwrap_or(target);
                (target_root == type_name).then_some(*executable_index)
            })
    }

    fn local_type_name_for_receiver_type(&self, ty: &TypeRefIr) -> Option<&str> {
        let TypeRefIr::LocalType { type_index } = ty else {
            return None;
        };
        self.type_indices
            .iter()
            .find_map(|(name, index)| (*index == *type_index).then_some(name.as_str()))
    }

    fn lower_static_call_target(&self, callee: &Expr) -> Result<CallTargetIr> {
        let path = expr_path(callee).ok_or_else(|| unsupported_call(callee))?;
        let root = path.split('.').next().unwrap_or(path.as_str());
        if let Some(service_path) = path.strip_prefix("root.") {
            if is_official_std_module_path(self.module_path) {
                return Ok(CallTargetIr::ExternalServiceSymbol {
                    symbol: service_symbol_ref(
                        self.module_path,
                        &package_scoped_root_path(self.module_path, service_path),
                    ),
                });
            }
            let package_path = package_scoped_root_path(self.module_path, service_path);
            if let Some((dependency_ref, symbol_path)) = self.package_symbol_path(&package_path) {
                return self.package_operation_call_target(
                    &package_path,
                    dependency_ref,
                    symbol_path,
                );
            }
            return Ok(CallTargetIr::ExternalServiceSymbol {
                symbol: service_symbol_ref(self.module_path, service_path),
            });
        }
        if let Some((dependency_ref, source_call_path)) =
            self.service_dependency_operation_path(&path)
        {
            return self.service_dependency_operation_call_target(
                &path,
                dependency_ref,
                source_call_path,
            );
        }
        if let Some(target) = shared_native_alias_target(&path) {
            return Ok(CallTargetIr::Native {
                target: native_target_from_symbol(target),
            });
        }
        if prelude_registry().native_binding_key(&path).is_some() {
            return Ok(CallTargetIr::Native {
                target: native_target_from_symbol(&path),
            });
        }
        if is_builtin_call_root(root) {
            return Ok(CallTargetIr::Builtin { op: path });
        }
        if prelude_registry().is_native_symbol(&path) {
            return Ok(CallTargetIr::Native {
                target: native_target_from_symbol(&path),
            });
        }
        if let Some(executable_index) = self.executable_indices.get(&path) {
            return Ok(CallTargetIr::LocalExecutable {
                executable_index: *executable_index,
            });
        }
        if let Some(local_symbol) = path.strip_prefix(&format!("{}.", self.module_path)) {
            if let Some(executable_index) = self.executable_indices.get(local_symbol) {
                return Ok(CallTargetIr::LocalExecutable {
                    executable_index: *executable_index,
                });
            }
        }
        if let Some((dependency_ref, symbol_path)) = self.package_symbol_path(&path) {
            return self.package_operation_call_target(&path, dependency_ref, symbol_path);
        }
        if let Some((dependency_ref, source_call_path)) =
            self.service_dependency_operation_path(&path)
        {
            return self.service_dependency_operation_call_target(
                &path,
                dependency_ref,
                source_call_path,
            );
        }
        if !path.contains('.') {
            return Err(unsupported_file_ir_callee(&path));
        }
        Ok(CallTargetIr::ExternalServiceSymbol {
            symbol: service_symbol_ref(self.module_path, &path),
        })
    }

    fn package_symbol_path(&self, path: &str) -> Option<(String, String)> {
        let (root, _) = path.split_once('.')?;
        if root == "root" {
            return None;
        }
        PackageExportResolver::new(self.package_aliases)
            .resolve_package_symbol_path(path)
            .map(|symbol| (symbol.dependency_ref, symbol.symbol_path))
    }

    fn package_operation_call_target(
        &self,
        source_path: &str,
        dependency_ref: String,
        source_call_path: String,
    ) -> Result<CallTargetIr> {
        let operation = self
            .package_operations
            .resolve(&dependency_ref, &source_call_path)?
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "package dependency `{dependency_ref}` does not export public operation `{source_call_path}` for source call `{source_path}`"
                ))
            })?
            .clone();
        Ok(CallTargetIr::PackageSymbol {
            package_ref: PackageRefIr::Dependency { dependency_ref },
            operation,
        })
    }

    fn service_dependency_operation_call_target(
        &self,
        source_path: &str,
        dependency_ref: String,
        source_call_path: String,
    ) -> Result<CallTargetIr> {
        let operation = self
            .service_dependency_operations
            .resolve(&dependency_ref, &source_call_path)?
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "service dependency `{dependency_ref}` does not export public operation `{source_call_path}` for source call `{source_path}`"
                ))
            })?
            .clone();
        Ok(CallTargetIr::ServiceDependencySymbol {
            symbol: ServiceDependencySymbolRef {
                dependency_ref,
                operation,
            },
        })
    }

    fn remote_public_instance_direct_call_target(
        &self,
        object: &Expr,
        method_name: &str,
    ) -> Result<Option<CallTargetIr>> {
        let Expr::RemotePublicInstanceSource(source) = object else {
            return Ok(None);
        };
        let source_path = format!(
            "{}/{}.{}",
            source.dependency_ref, source.public_instance_key, method_name
        );
        let source_call_path = format!("{}.{}", source.public_instance_key, method_name);
        self.service_dependency_operation_call_target(
            &source_path,
            source.dependency_ref.clone(),
            source_call_path,
        )
        .map(Some)
    }

    fn service_dependency_operation_path(&self, path: &str) -> Option<(String, String)> {
        let (root, operation) = path.split_once('.')?;
        if operation.is_empty() || !self.service_dependency_aliases.contains(root) {
            return None;
        }
        Some((root.to_string(), operation.to_string()))
    }

    fn is_receiver_call(&self, object: &Expr) -> bool {
        match object {
            Expr::Identifier(name) => self.bindings.contains_key(name),
            Expr::Field { .. } => expr_path(object)
                .and_then(|path| path.split('.').next().map(str::to_string))
                .is_some_and(|root| self.bindings.contains_key(&root)),
            Expr::Call { .. } => expr_path(object).is_none(),
            _ => true,
        }
    }

    fn builtin_receiver_op_for_type(
        ty: &TypeRefIr,
        method_name: &str,
    ) -> Option<BuiltinReceiverOp> {
        let root = runtime_receiver_root_from_type_ref(ty)?;
        builtin_receiver_op_by_name(&root, method_name)
    }

    fn is_actor_ref_receiver_type(ty: &TypeRefIr) -> bool {
        match ty {
            TypeRefIr::Native { name, .. } if name == "ActorRef" => true,
            TypeRefIr::PackageSymbol { symbol } if is_actor_ref_package_symbol(symbol) => true,
            _ => false,
        }
    }

    fn lower_object_literal_key(&mut self, key: &ObjectLiteralKey) -> Result<String> {
        match key {
            ObjectLiteralKey::Name(key) => Ok(key.clone()),
        }
    }

    fn lower_match_arm(&mut self, arm: &skiff_syntax::ast::MatchArm) -> Result<MatchArmIr> {
        let label = self.next_block_label("match_arm");
        self.push_scope();
        let pattern = self.lower_pattern_and_bind(&arm.pattern)?;
        let mut block = BlockIr {
            label: label.clone(),
            statements: Vec::new(),
        };
        for stmt in &arm.body.statements {
            block.statements.push(self.lower_stmt(stmt)?);
        }
        self.pop_scope();
        self.body.blocks.push(block);
        Ok(MatchArmIr {
            pattern,
            body: label,
        })
    }

    fn lower_pattern_and_bind(
        &mut self,
        pattern: &skiff_syntax::ast::Pattern,
    ) -> Result<PatternIr> {
        Ok(match pattern {
            skiff_syntax::ast::Pattern::Wildcard => PatternIr::Wildcard,
            skiff_syntax::ast::Pattern::Literal(value) => PatternIr::Literal {
                value: lower_literal(value)?,
            },
            skiff_syntax::ast::Pattern::Binding(name) => PatternIr::Binding {
                slot: self.declare_slot(name, SlotKind::Pattern, false)?,
            },
            skiff_syntax::ast::Pattern::Nominal { name, .. } => {
                return Err(CompileError::Semantic(format!(
                    "nominal pattern `{name}` cannot match an erased runtime value; use a record, literal, binding, or wildcard pattern"
                )));
            }
            skiff_syntax::ast::Pattern::Record { fields } => {
                self.declare_pattern_fields(fields)?;
                PatternIr::Wildcard
            }
            skiff_syntax::ast::Pattern::Or(patterns) => {
                if let Some(pattern) = patterns.first() {
                    self.lower_pattern_and_bind(pattern)?
                } else {
                    PatternIr::Wildcard
                }
            }
        })
    }

    fn declare_pattern_fields(&mut self, fields: &[skiff_syntax::ast::PatternField]) -> Result<()> {
        for field in fields {
            match &field.pattern {
                Some(pattern) => {
                    self.lower_pattern_and_bind(pattern)?;
                }
                None => {
                    self.declare_slot(&field.name, SlotKind::Pattern, false)?;
                }
            }
        }
        Ok(())
    }

    fn exception_slot(&self, expr: &Expr) -> Result<u32> {
        let Expr::Identifier(name) = expr else {
            return Err(unsupported(
                "rethrow in typed File IR requires an exception slot identifier",
            ));
        };
        self.bindings
            .get(name)
            .map(|binding| binding.slot)
            .ok_or_else(|| CompileError::Semantic(format!("unresolved rethrow exception `{name}`")))
    }

    fn throw_payload_type(&self, value: &Expr, offset: u32) -> Result<TypeRefIr> {
        if let Some((_, ty)) = self.expression_type_at_offset(offset) {
            return Ok(ty);
        }
        if let Some(ty) = self.static_record_constructor_type(value) {
            return Ok(ty);
        }
        if self.expression_types.is_some() {
            if let Some(ty) = self.infer_expr_type_ir(value) {
                return Ok(ty);
            }
        }
        Err(CompileError::Semantic(
            "throw lowering requires a static payload type fact".to_string(),
        ))
    }

    fn static_record_constructor_type(&self, value: &Expr) -> Option<TypeRefIr> {
        let Expr::Record {
            type_name,
            type_args,
            ..
        } = value
        else {
            return None;
        };
        lower_named_type(
            type_name,
            type_args,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.value_type_context(),
        )
        .ok()
    }

    pub(super) fn push_stmt(&mut self, stmt: StmtIr) -> StmtRefIr {
        let statement = self.body.statements.len() as u32;
        self.body.statements.push(stmt);
        StmtRefIr { statement }
    }

    pub(super) fn push_expr(&mut self, expr: ExprIr) -> ExprRefIr {
        let expression = self.body.expressions.len() as u32;
        self.body.expressions.push(expr);
        ExprRefIr { expression }
    }
}

fn unsupported_call(callee: &Expr) -> CompileError {
    let callee = expr_path(callee).unwrap_or_else(|| "<complex callee>".to_string());
    unsupported(format!(
        "function call callee `{callee}` is not resolved by the File IR unit emitter yet"
    ))
}

fn unsupported_file_ir_callee(callee: &str) -> CompileError {
    unsupported(format!(
        "function call callee `{callee}` is not resolved by the File IR unit emitter"
    ))
}

fn unsupported(message: impl Into<String>) -> CompileError {
    CompileError::Semantic(message.into())
}

fn expression_key_offset(key: &ExpressionKey, offset: u32, purpose: &str) -> Result<ExpressionKey> {
    let preorder_index = key.preorder_index().checked_add(offset).ok_or_else(|| {
        CompileError::Semantic(format!(
            "{purpose} lowering cannot derive child ExpressionKey from {:?}: preorder index overflow",
            key
        ))
    })?;
    Ok(ExpressionKey::new(
        key.module_path().to_string(),
        key.owner().clone(),
        preorder_index,
    ))
}

fn is_std_http_json_native_target(target: &NativeTarget) -> bool {
    matches!(
        target.binding_key.as_deref(),
        Some("std.http.response.json" | "std.http.response.jsonWithHeaders")
    ) || (target.namespace == "std.http"
        && matches!(target.symbol.as_str(), "json" | "jsonWithHeaders"))
}

fn expr_preorder_node_count(expr: &Expr) -> u32 {
    match expr {
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::RemotePublicInstanceSource(_)
        | Expr::DbOperation(_)
        | Expr::DbQuery(_) => 1,
        Expr::Unary { expr, .. }
        | Expr::Generic { callee: expr, .. }
        | Expr::InterfaceBox { value: expr, .. }
        | Expr::Field { object: expr, .. }
        | Expr::Throw { value: expr }
        | Expr::Rethrow { exception: expr }
        | Expr::Catch { try_expr: expr, .. } => 1 + expr_preorder_node_count(expr),
        Expr::Binary { left, right, .. } => {
            1 + expr_preorder_node_count(left) + expr_preorder_node_count(right)
        }
        Expr::Call { callee, args } => {
            1 + expr_preorder_node_count(callee)
                + args.iter().map(expr_preorder_node_count).sum::<u32>()
        }
        Expr::Record { fields, .. } => {
            1 + fields
                .iter()
                .map(|(_, value)| expr_preorder_node_count(value))
                .sum::<u32>()
        }
        Expr::ObjectLiteral { entries } => {
            1 + entries
                .iter()
                .map(|entry| expr_preorder_node_count(&entry.value))
                .sum::<u32>()
        }
        Expr::Patch { operations, .. } => {
            1 + operations
                .iter()
                .map(|operation| match operation {
                    PatchOperation::Set { value, .. } | PatchOperation::Inc { value, .. } => {
                        expr_preorder_node_count(value)
                    }
                })
                .sum::<u32>()
        }
        Expr::DbTransaction(transaction) => 1 + block_preorder_node_count(&transaction.body),
        Expr::DbLeaseClaim(claim) => {
            1 + expr_preorder_node_count(&claim.key) + block_preorder_node_count(&claim.body)
        }
        Expr::DbLeaseRead(read) => 1 + expr_preorder_node_count(&read.key),
    }
}

fn block_preorder_node_count(block: &skiff_syntax::ast::Block) -> u32 {
    block
        .statements
        .iter()
        .map(stmt_preorder_node_count)
        .sum::<u32>()
}

fn stmt_preorder_node_count(stmt: &Stmt) -> u32 {
    match stmt {
        Stmt::Assert { condition, .. } => expr_preorder_node_count(condition),
        Stmt::Let { value, .. } => expr_preorder_node_count(value),
        Stmt::Assign { target, value } => {
            expr_preorder_node_count(target) + expr_preorder_node_count(value)
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            expr_preorder_node_count(condition)
                + block_preorder_node_count(then_block)
                + else_block
                    .as_ref()
                    .map(block_preorder_node_count)
                    .unwrap_or_default()
        }
        Stmt::For { iterable, body, .. } => {
            expr_preorder_node_count(iterable) + block_preorder_node_count(body)
        }
        Stmt::Match { value, arms } => {
            expr_preorder_node_count(value)
                + arms
                    .iter()
                    .map(|arm| block_preorder_node_count(&arm.body))
                    .sum::<u32>()
        }
        Stmt::DbTransaction { body } => block_preorder_node_count(body),
        Stmt::Throw { value }
        | Stmt::Rethrow { exception: value }
        | Stmt::Emit(value)
        | Stmt::Spawn { call: value }
        | Stmt::Expr(value) => expr_preorder_node_count(value),
        Stmt::Return(value) => value
            .as_ref()
            .map(expr_preorder_node_count)
            .unwrap_or_default(),
        Stmt::Break | Stmt::Continue => 0,
    }
}

pub(super) fn expr_path(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(name) => Some(name.clone()),
        Expr::RemotePublicInstanceSource(source) => Some(format!(
            "{}/{}",
            source.dependency_ref, source.public_instance_key
        )),
        Expr::Field { object, field } => Some(format!("{}.{}", expr_path(object)?, field)),
        Expr::Generic { callee, .. } => expr_path(callee),
        _ => None,
    }
}

fn is_actor_ref_package_symbol(symbol: &PackageSymbolRef) -> bool {
    if !matches!(
        symbol.symbol_path.as_str(),
        "actor.ActorRef" | "std.actor.ActorRef"
    ) {
        return false;
    }
    is_std_package_ref(&symbol.package)
}

fn is_std_package_ref(package: &PackageRefIr) -> bool {
    match package {
        PackageRefIr::PackageId { package_id } => package_id == SKIFF_STD_PUBLICATION_ID,
        PackageRefIr::Dependency { dependency_ref } => dependency_ref == "std",
    }
}

pub(super) fn is_builtin_call_root(root: &str) -> bool {
    matches!(
        root,
        "Array"
            | "Date"
            | "Map"
            | "object"
            | "string"
            | "number"
            | "bytes"
            | "json"
            | "config"
            | "db"
            | "root"
    )
}

pub(super) fn is_db_builtin_op(op: &str) -> bool {
    matches!(
        op,
        "db.get"
            | "db.require"
            | "db.exists"
            | "db.create"
            | "db.createMany"
            | "db.create_many"
            | "db.append"
            | "db.appendMany"
            | "db.append_many"
            | "db.upsert"
            | "db.findMany"
            | "db.find_many"
            | "db.count"
            | "db.transaction"
    )
}

pub(super) fn block_contains_return_stmt(block: &skiff_syntax::ast::Block) -> bool {
    block.statements.iter().any(stmt_contains_return_stmt)
}

fn stmt_contains_return_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(_) => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            block_contains_return_stmt(then_block)
                || else_block.as_ref().is_some_and(block_contains_return_stmt)
        }
        Stmt::For { body, .. } | Stmt::DbTransaction { body } => block_contains_return_stmt(body),
        Stmt::Match { arms, .. } => arms.iter().any(|arm| block_contains_return_stmt(&arm.body)),
        Stmt::Assert { .. }
        | Stmt::Let { .. }
        | Stmt::Assign { .. }
        | Stmt::Throw { .. }
        | Stmt::Rethrow { .. }
        | Stmt::Emit(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Spawn { .. }
        | Stmt::Expr(_) => false,
    }
}

fn interface_method_slot_signature_params(
    signature: &LoweredExecutableSignature,
    concrete_type: &TypeRefIr,
) -> Vec<FunctionTypeParamIr> {
    if signature.self_type.is_some() {
        let mut params = Vec::with_capacity(signature.params.len() + 1);
        params.push(FunctionTypeParamIr {
            name: "self".to_string(),
            ty: concrete_type.clone(),
        });
        params.extend(signature.params.clone());
        return params;
    }

    let mut params = signature.params.clone();
    if let Some(first) = params.first_mut() {
        if first.name == "self" {
            first.ty = concrete_type.clone();
        }
    }
    params
}

fn lower_literal(literal: &Literal) -> Result<LiteralIr> {
    Ok(match literal {
        Literal::Null => LiteralIr::Null,
        Literal::Bool(value) => LiteralIr::Bool { value: *value },
        Literal::Number(value) => LiteralIr::Number {
            value: Number::from_f64(*value).ok_or_else(|| {
                CompileError::Semantic(format!(
                    "invalid non-finite number literal `{value}` in File IR unit expression"
                ))
            })?,
        },
        Literal::String(value) => LiteralIr::String {
            value: value.clone(),
        },
    })
}

fn lower_unary_op(op: UnaryOp) -> UnaryOpIr {
    match op {
        UnaryOp::Not => UnaryOpIr::Not,
    }
}

fn lower_binary_op(op: BinaryOp) -> BinaryOpIr {
    match op {
        BinaryOp::Add => BinaryOpIr::Add,
        BinaryOp::Sub => BinaryOpIr::Subtract,
        BinaryOp::Mul => BinaryOpIr::Multiply,
        BinaryOp::Div => BinaryOpIr::Divide,
        BinaryOp::Eq => BinaryOpIr::Equal,
        BinaryOp::Ne => BinaryOpIr::NotEqual,
        BinaryOp::Lt => BinaryOpIr::LessThan,
        BinaryOp::Le => BinaryOpIr::LessThanOrEqual,
        BinaryOp::Gt => BinaryOpIr::GreaterThan,
        BinaryOp::Ge => BinaryOpIr::GreaterThanOrEqual,
        BinaryOp::And => BinaryOpIr::And,
        BinaryOp::Or => BinaryOpIr::Or,
    }
}
