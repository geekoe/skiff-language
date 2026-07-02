use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::{
    BlockIr, CallIr, CallTargetIr, ConstDeclarationIr, ConstIr, ExecutableBody,
    ExecutableDeclarationIr, ExecutableIr, ExecutableKind, ExprIr, FileIrUnit, FunctionTypeParamIr,
    ParamIr, SlotKind, SlotLayout, StmtIr,
};
use skiff_compiler_source::{
    semantic::{
        executable_symbol, impl_method_declaration_name, ExecutableIndex, InterfaceSemantics,
    },
    ExpressionOwnerKey, ExpressionTypeModel, LocalDbObjectIndex, PackageInterfaceMethodIndex,
    PublicationDbMetadataIndex, PublicationTypeSymbolIndex, TypeResolutionModel,
};
use skiff_syntax::{
    ast::{ConstDecl, FunctionDecl, ImplDecl, Stmt, TypeRef},
    error::{CompileError, Result},
    type_syntax::generic_parts,
};

use super::{
    callable_return_types::CallableReturnType,
    db_lowering::{DbMetadataIr, LoweredPublicationDbMetadataIndex},
    dependency_operation_indexes::{PackageOperationIndex, ServiceDependencyOperationIndex},
    function_lowering::{
        native_target_from_symbol, BindingReadonlyFlags, FunctionLowerer, LocalTypeFieldIndex,
        LoweredExecutableSignature,
    },
    source_unit_lowering::{push_source_span, source_span_ref, symbol, type_param_scope},
    suspend_analysis::SuspendIndex,
    type_lowering::{
        bare_type_name, is_file_ir_builtin_generic_type, is_file_ir_builtin_type, lower_type_ref,
        type_root, TypeLoweringContext,
    },
};

fn generic_type_params(name: &str, type_indices: &BTreeMap<String, u32>) -> Vec<String> {
    generic_parts(name)
        .map(|parts| {
            parts
                .args
                .iter()
                .map(|arg| arg.trim())
                .filter(|arg| {
                    !arg.is_empty()
                        && !type_indices.contains_key(*arg)
                        && !is_file_ir_builtin_type(arg)
                        && !is_file_ir_builtin_generic_type(arg)
                        && arg
                            .chars()
                            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_const_declarations(
    constants: &[ConstDecl],
    const_indices: &BTreeMap<String, u32>,
    executable_indices: &BTreeMap<String, u32>,
    db_metadata: &BTreeMap<String, DbMetadataIr>,
    publication_db_metadata: &PublicationDbMetadataIndex,
    lowered_publication_db_metadata: &LoweredPublicationDbMetadataIndex,
    type_indices: &BTreeMap<String, u32>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_interface_methods: &PackageInterfaceMethodIndex,
    package_operations: &PackageOperationIndex,
    service_dependency_operations: &ServiceDependencyOperationIndex,
    external_type_symbols: &PublicationTypeSymbolIndex,
    service_dependency_aliases: &BTreeSet<String>,
    module_path: &str,
    local_db_objects: &LocalDbObjectIndex,
    interface_semantics: &InterfaceSemantics,
    source_alias_targets: &BTreeMap<String, String>,
    type_resolution: &TypeResolutionModel,
    expression_types: Option<&ExpressionTypeModel>,
    callable_return_types: &BTreeMap<String, CallableReturnType>,
    local_type_fields: &LocalTypeFieldIndex,
    executable_signatures: &BTreeMap<u32, LoweredExecutableSignature>,
    unit: &mut FileIrUnit,
    next_span_id: &mut u64,
) -> Result<()> {
    for constant in constants {
        let const_index = const_indices.get(&constant.name).copied().ok_or_else(|| {
            CompileError::Semantic(format!("missing local const index for `{}`", constant.name))
        })?;
        let ty = constant
            .ty
            .as_ref()
            .map(|ty| {
                lower_type_ref(
                    ty,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    TypeLoweringContext::value(),
                )
            })
            .transpose()?
            .ok_or_else(|| {
                unsupported(format!(
                    "top-level const `{}` requires an explicit type annotation in File IR units",
                    constant.name
                ))
            })?;
        let source_span = source_span_ref(constant.span);
        let body = lower_const_initializer_body(
            constant,
            const_indices,
            executable_indices,
            db_metadata,
            publication_db_metadata,
            lowered_publication_db_metadata,
            type_indices,
            package_aliases,
            package_interface_methods,
            package_operations,
            service_dependency_operations,
            external_type_symbols,
            service_dependency_aliases,
            module_path,
            local_db_objects,
            interface_semantics,
            source_alias_targets,
            type_resolution,
            expression_types,
            callable_return_types,
            local_type_fields,
            executable_signatures,
        )?;
        unit.constants.push(ConstIr {
            name: constant.name.clone(),
            ty: ty.clone(),
            body,
            source_span: Some(source_span.clone()),
        });
        unit.declarations.constants.insert(
            constant.name.clone(),
            ConstDeclarationIr {
                const_index,
                symbol: symbol(module_path, &constant.name),
                ty,
                source_span: Some(source_span),
            },
        );
        // link_targets recomputed post-lowering (see `LoweredPublication::lower`).
        push_source_span(
            &mut unit.source_map.spans,
            next_span_id,
            "const",
            &constant.name,
            constant.span,
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_const_initializer_body(
    constant: &ConstDecl,
    const_indices: &BTreeMap<String, u32>,
    executable_indices: &BTreeMap<String, u32>,
    db_metadata: &BTreeMap<String, DbMetadataIr>,
    publication_db_metadata: &PublicationDbMetadataIndex,
    lowered_publication_db_metadata: &LoweredPublicationDbMetadataIndex,
    type_indices: &BTreeMap<String, u32>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_interface_methods: &PackageInterfaceMethodIndex,
    package_operations: &PackageOperationIndex,
    service_dependency_operations: &ServiceDependencyOperationIndex,
    external_type_symbols: &PublicationTypeSymbolIndex,
    service_dependency_aliases: &BTreeSet<String>,
    module_path: &str,
    local_db_objects: &LocalDbObjectIndex,
    interface_semantics: &InterfaceSemantics,
    source_alias_targets: &BTreeMap<String, String>,
    type_resolution: &TypeResolutionModel,
    expression_types: Option<&ExpressionTypeModel>,
    callable_return_types: &BTreeMap<String, CallableReturnType>,
    local_type_fields: &LocalTypeFieldIndex,
    executable_signatures: &BTreeMap<u32, LoweredExecutableSignature>,
) -> Result<ExecutableBody> {
    let mut lowerer = FunctionLowerer::new(
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
        BTreeSet::new(),
        Some(ExpressionOwnerKey::Const(constant.name.clone())),
        interface_semantics,
        type_resolution,
        expression_types,
        callable_return_types,
        local_type_fields,
        executable_signatures,
    );
    let value = lowerer.lower_expr(&constant.value)?;
    let mut entry = BlockIr {
        label: "entry".to_string(),
        statements: Vec::new(),
    };
    entry
        .statements
        .push(lowerer.push_stmt(StmtIr::Return { value: Some(value) }));
    lowerer.body.blocks.push(entry);
    Ok(lowerer.body)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lowered_executable_signatures(
    functions: &[FunctionDecl],
    impls: &[ImplDecl],
    executable_index: &ExecutableIndex,
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
) -> Result<BTreeMap<u32, LoweredExecutableSignature>> {
    let mut signatures = BTreeMap::new();
    for function in functions {
        let index = executable_index
            .entry(&function.name)
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "missing semantic executable index for `{}`",
                    function.name
                ))
            })?
            .executable_index;
        signatures.insert(
            index,
            lower_executable_signature(
                function,
                &function.params,
                &[],
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
            )?,
        );
    }

    for implementation in impls {
        let impl_type_params = generic_type_params(&implementation.target, type_indices);
        for method in &implementation.method_bodies {
            let name = impl_method_declaration_name(&implementation.target, &method.name);
            let index = executable_index
                .entry(&name)
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "missing semantic executable index for `{name}`"
                    ))
                })?
                .executable_index;
            signatures.insert(
                index,
                lower_executable_signature(
                    method,
                    &method.params,
                    &impl_type_params,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                )?,
            );
        }
    }

    Ok(signatures)
}

#[allow(clippy::too_many_arguments)]
fn lower_executable_signature(
    function: &FunctionDecl,
    params_source: &[skiff_syntax::ast::Param],
    inherited_type_params: &[String],
    type_indices: &BTreeMap<String, u32>,
    local_db_objects: &LocalDbObjectIndex,
    publication_db_metadata: &PublicationDbMetadataIndex,
    package_aliases: &BTreeMap<String, Vec<String>>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    source_alias_targets: &BTreeMap<String, String>,
) -> Result<LoweredExecutableSignature> {
    let type_params = type_param_scope(inherited_type_params.iter(), function.type_params.iter());
    let type_context = TypeLoweringContext::value_with_type_params(&type_params);
    let self_type = function
        .implicit_self
        .as_ref()
        .map(|ty| {
            lower_type_ref(
                ty,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                type_context,
            )
        })
        .transpose()?;
    let params = params_source
        .iter()
        .map(|param| {
            Ok(FunctionTypeParamIr {
                name: param.name.clone(),
                ty: lower_type_ref(
                    &param.ty,
                    type_indices,
                    local_db_objects,
                    publication_db_metadata,
                    package_aliases,
                    external_type_symbols,
                    source_alias_targets,
                    type_context,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let return_type = lower_type_ref(
        &function.return_type,
        type_indices,
        local_db_objects,
        publication_db_metadata,
        package_aliases,
        external_type_symbols,
        source_alias_targets,
        type_context,
    )?;
    Ok(LoweredExecutableSignature {
        params,
        return_type,
        self_type,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_executables(
    functions: &[FunctionDecl],
    impls: &[ImplDecl],
    db_metadata: &BTreeMap<String, DbMetadataIr>,
    publication_db_metadata: &PublicationDbMetadataIndex,
    lowered_publication_db_metadata: &LoweredPublicationDbMetadataIndex,
    suspend_index: &SuspendIndex,
    executable_index: &ExecutableIndex,
    const_indices: &BTreeMap<String, u32>,
    type_indices: &BTreeMap<String, u32>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    service_dependency_aliases: &BTreeSet<String>,
    module_path: &str,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_interface_methods: &PackageInterfaceMethodIndex,
    package_operations: &PackageOperationIndex,
    service_dependency_operations: &ServiceDependencyOperationIndex,
    local_db_objects: &LocalDbObjectIndex,
    interface_semantics: &InterfaceSemantics,
    source_alias_targets: &BTreeMap<String, String>,
    type_resolution: &TypeResolutionModel,
    expression_types: Option<&ExpressionTypeModel>,
    callable_return_types: &BTreeMap<String, CallableReturnType>,
    local_type_fields: &LocalTypeFieldIndex,
    executable_signatures: &BTreeMap<u32, LoweredExecutableSignature>,
    unit: &mut FileIrUnit,
    next_span_id: &mut u64,
) -> Result<()> {
    let executable_indices = executable_index.indices();
    for function in functions {
        let name = function.name.clone();
        let may_suspend = suspend_index.function_may_suspend(&name);
        let symbol = executable_symbol(module_path, &name);
        let current_index = semantic_executable_index(executable_index, &name, module_path, unit)?;
        push_executable(
            function,
            ExecutableKind::Function,
            name,
            symbol,
            module_path,
            current_index,
            function.exported,
            &[],
            ExpressionOwnerKey::Function(function.name.clone()),
            db_metadata,
            publication_db_metadata,
            lowered_publication_db_metadata,
            may_suspend,
            &executable_indices,
            const_indices,
            type_indices,
            external_type_symbols,
            service_dependency_aliases,
            package_aliases,
            package_interface_methods,
            package_operations,
            service_dependency_operations,
            local_db_objects,
            interface_semantics,
            source_alias_targets,
            type_resolution,
            expression_types,
            callable_return_types,
            local_type_fields,
            executable_signatures,
            unit,
            next_span_id,
        )?;
    }

    for implementation in impls {
        let body_names = implementation
            .method_bodies
            .iter()
            .map(|method| method.name.as_str())
            .collect::<BTreeSet<_>>();
        let missing_bodies = implementation
            .methods
            .iter()
            .filter(|method| !method.is_native && !body_names.contains(method.name.as_str()))
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>();
        if !missing_bodies.is_empty() {
            return Err(unsupported(format!(
                "impl `{}` contains bodyless or unparsable methods that cannot be emitted as File IR units: {}",
                implementation.target,
                missing_bodies.join(", ")
            )));
        }

        let impl_type_params = generic_type_params(&implementation.target, type_indices);
        for method in &implementation.method_bodies {
            let name = impl_method_declaration_name(&implementation.target, &method.name);
            let symbol = executable_symbol(module_path, &name);
            let current_index =
                semantic_executable_index(executable_index, &name, module_path, unit)?;
            push_executable(
                method,
                ExecutableKind::ImplMethod,
                name,
                symbol,
                module_path,
                current_index,
                implementation.exported,
                &impl_type_params,
                ExpressionOwnerKey::ImplMethod {
                    type_name: implementation.target.clone(),
                    method: method.name.clone(),
                },
                db_metadata,
                publication_db_metadata,
                lowered_publication_db_metadata,
                suspend_index.method_may_suspend(&implementation.target, &method.name),
                &executable_indices,
                const_indices,
                type_indices,
                external_type_symbols,
                service_dependency_aliases,
                package_aliases,
                package_interface_methods,
                package_operations,
                service_dependency_operations,
                local_db_objects,
                interface_semantics,
                source_alias_targets,
                type_resolution,
                expression_types,
                callable_return_types,
                local_type_fields,
                executable_signatures,
                unit,
                next_span_id,
            )?;
        }
    }

    Ok(())
}

fn semantic_executable_index(
    executable_index: &ExecutableIndex,
    declaration_name: &str,
    module_path: &str,
    unit: &FileIrUnit,
) -> Result<u32> {
    let entry = executable_index.entry(declaration_name).ok_or_else(|| {
        CompileError::Semantic(format!(
            "missing semantic executable index for `{declaration_name}` in module {module_path}"
        ))
    })?;
    assert_executable_position(
        declaration_name,
        entry.executable_index,
        unit.executables.len(),
        module_path,
    )?;
    Ok(entry.executable_index)
}

fn assert_executable_position(
    declaration_name: &str,
    semantic_index: u32,
    emit_position: usize,
    module_path: &str,
) -> Result<()> {
    let emit_position = u32::try_from(emit_position).map_err(|_| {
        CompileError::Semantic(format!(
            "too many executables in module {module_path} to fit semantic executable index"
        ))
    })?;
    if semantic_index != emit_position {
        return Err(CompileError::Semantic(format!(
            "executable `{declaration_name}` in module {module_path} is emitted at index {emit_position}, but semantic executable index is {semantic_index}"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_executable(
    function: &FunctionDecl,
    kind: ExecutableKind,
    declaration_name: String,
    executable_symbol: String,
    module_path: &str,
    current_index: u32,
    _exported: bool,
    inherited_type_params: &[String],
    owner: ExpressionOwnerKey,
    db_metadata: &BTreeMap<String, DbMetadataIr>,
    publication_db_metadata: &PublicationDbMetadataIndex,
    lowered_publication_db_metadata: &LoweredPublicationDbMetadataIndex,
    may_suspend: bool,
    executable_indices: &BTreeMap<String, u32>,
    const_indices: &BTreeMap<String, u32>,
    type_indices: &BTreeMap<String, u32>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    service_dependency_aliases: &BTreeSet<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_interface_methods: &PackageInterfaceMethodIndex,
    package_operations: &PackageOperationIndex,
    service_dependency_operations: &ServiceDependencyOperationIndex,
    local_db_objects: &LocalDbObjectIndex,
    interface_semantics: &InterfaceSemantics,
    source_alias_targets: &BTreeMap<String, String>,
    type_resolution: &TypeResolutionModel,
    expression_types: Option<&ExpressionTypeModel>,
    callable_return_types: &BTreeMap<String, CallableReturnType>,
    local_type_fields: &LocalTypeFieldIndex,
    executable_signatures: &BTreeMap<u32, LoweredExecutableSignature>,
    unit: &mut FileIrUnit,
    next_span_id: &mut u64,
) -> Result<()> {
    let executable = lower_function_with_params(
        function,
        kind,
        executable_symbol.clone(),
        module_path,
        &function.params,
        inherited_type_params,
        owner,
        db_metadata,
        publication_db_metadata,
        lowered_publication_db_metadata,
        executable_indices,
        const_indices,
        type_indices,
        external_type_symbols,
        service_dependency_aliases,
        package_aliases,
        package_interface_methods,
        package_operations,
        service_dependency_operations,
        local_db_objects,
        interface_semantics,
        source_alias_targets,
        may_suspend,
        type_resolution,
        expression_types,
        callable_return_types,
        local_type_fields,
        executable_signatures,
    )?;
    let source_span = source_span_ref(function.span);

    unit.declarations.executables.insert(
        declaration_name.clone(),
        ExecutableDeclarationIr {
            executable_index: current_index,
            symbol: executable_symbol,
            source_span: Some(source_span),
        },
    );
    // link_targets recomputed post-lowering (see `LoweredPublication::lower`).
    unit.executables.push(executable);
    push_source_span(
        &mut unit.source_map.spans,
        next_span_id,
        "function",
        &declaration_name,
        function.span,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_function_with_params(
    function: &FunctionDecl,
    kind: ExecutableKind,
    executable_symbol: String,
    module_path: &str,
    params_source: &[skiff_syntax::ast::Param],
    inherited_type_params: &[String],
    owner: ExpressionOwnerKey,
    db_metadata: &BTreeMap<String, DbMetadataIr>,
    publication_db_metadata: &PublicationDbMetadataIndex,
    lowered_publication_db_metadata: &LoweredPublicationDbMetadataIndex,
    executable_indices: &BTreeMap<String, u32>,
    const_indices: &BTreeMap<String, u32>,
    type_indices: &BTreeMap<String, u32>,
    external_type_symbols: &PublicationTypeSymbolIndex,
    service_dependency_aliases: &BTreeSet<String>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_interface_methods: &PackageInterfaceMethodIndex,
    package_operations: &PackageOperationIndex,
    service_dependency_operations: &ServiceDependencyOperationIndex,
    local_db_objects: &LocalDbObjectIndex,
    interface_semantics: &InterfaceSemantics,
    source_alias_targets: &BTreeMap<String, String>,
    may_suspend: bool,
    type_resolution: &TypeResolutionModel,
    expression_types: Option<&ExpressionTypeModel>,
    callable_return_types: &BTreeMap<String, CallableReturnType>,
    local_type_fields: &LocalTypeFieldIndex,
    executable_signatures: &BTreeMap<u32, LoweredExecutableSignature>,
) -> Result<ExecutableIr> {
    validate_bare_return_statements(function, &executable_symbol)?;
    let type_params = type_param_scope(inherited_type_params.iter(), function.type_params.iter());
    let type_context = TypeLoweringContext::value_with_type_params(&type_params);
    let mut lowerer = FunctionLowerer::new(
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
        type_params.clone(),
        Some(owner),
        interface_semantics,
        type_resolution,
        expression_types,
        callable_return_types,
        local_type_fields,
        executable_signatures,
    );
    let self_type = function
        .implicit_self
        .as_ref()
        .map(|ty| {
            lower_type_ref(
                ty,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                type_context,
            )
        })
        .transpose()?;
    if self_type.is_some() {
        let self_type_text = function.implicit_self.as_ref().map(|ty| ty.name.clone());
        lowerer.declare_slot_with_type(
            "self",
            SlotKind::SelfValue,
            false,
            BindingReadonlyFlags::default(),
            self_type_text,
        )?;
    }

    let mut params = Vec::new();
    for param in params_source {
        let slot = lowerer.declare_slot_with_type(
            &param.name,
            SlotKind::Param,
            false,
            BindingReadonlyFlags::default(),
            Some(param.ty.name.clone()),
        )?;
        params.push(ParamIr {
            name: param.name.clone(),
            slot,
            ty: lower_type_ref(
                &param.ty,
                type_indices,
                local_db_objects,
                publication_db_metadata,
                package_aliases,
                external_type_symbols,
                source_alias_targets,
                type_context,
            )?,
        });
    }

    let mut entry = BlockIr {
        label: "entry".to_string(),
        statements: Vec::new(),
    };
    if function.is_native {
        let args = params
            .iter()
            .map(|param| lowerer.push_expr(ExprIr::LoadSlot { slot: param.slot }))
            .collect::<Vec<_>>();
        let type_args = function
            .type_params
            .iter()
            .enumerate()
            .map(|(index, name)| {
                Ok((
                    format!("T{index}"),
                    lower_type_ref(
                        &TypeRef { name: name.clone() },
                        type_indices,
                        local_db_objects,
                        publication_db_metadata,
                        package_aliases,
                        external_type_symbols,
                        source_alias_targets,
                        type_context,
                    )?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let call = lowerer.push_expr(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::Native {
                    target: native_target_from_symbol(&executable_symbol),
                },
                args,
                type_args,
                metadata: BTreeMap::new(),
            },
        });
        entry
            .statements
            .push(lowerer.push_stmt(StmtIr::Return { value: Some(call) }));
    } else {
        for stmt in &function.body.statements {
            entry.statements.push(lowerer.lower_stmt(stmt)?);
        }
    }
    lowerer.body.blocks.push(entry);
    let slots = SlotLayout {
        frame_size: lowerer.slots.len() as u32,
        slots: lowerer.slots,
    };

    Ok(ExecutableIr {
        kind,
        symbol: executable_symbol,
        type_params: function.type_params.clone(),
        params,
        return_type: lower_type_ref(
            &function.return_type,
            type_indices,
            local_db_objects,
            publication_db_metadata,
            package_aliases,
            external_type_symbols,
            source_alias_targets,
            type_context,
        )?,
        self_type,
        slots,
        may_suspend,
        body: lowerer.body,
        source_span: Some(source_span_ref(function.span)),
    })
}

fn validate_bare_return_statements(function: &FunctionDecl, executable_symbol: &str) -> Result<()> {
    if return_type_accepts_bare_return(&function.return_type.name)
        || !block_contains_bare_return(&function.body)
    {
        return Ok(());
    }

    Err(CompileError::Semantic(format!(
        "function `{executable_symbol}` returns `{}` but contains a bare return without a value",
        function.return_type.name
    )))
}

fn return_type_accepts_bare_return(return_type: &str) -> bool {
    matches!(
        bare_type_name(type_root(return_type)),
        "null" | "void" | "Stream"
    )
}

fn block_contains_bare_return(block: &skiff_syntax::ast::Block) -> bool {
    block.statements.iter().any(stmt_contains_bare_return)
}

fn stmt_contains_bare_return(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(None) => true,
        Stmt::Return(Some(_)) => false,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            block_contains_bare_return(then_block)
                || else_block.as_ref().is_some_and(block_contains_bare_return)
        }
        Stmt::For { body, .. } | Stmt::DbTransaction { body } => block_contains_bare_return(body),
        Stmt::Match { arms, .. } => arms.iter().any(|arm| block_contains_bare_return(&arm.body)),
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

fn unsupported(message: impl Into<String>) -> CompileError {
    CompileError::Semantic(message.into())
}
