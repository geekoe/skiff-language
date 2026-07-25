use std::collections::BTreeSet;

use skiff_artifact_model::PackageTypeRef;
use skiff_compiler_source::{
    SourceExecutableReceiver, SourceInterfaceMethodKey, SourceInterfaceRequirementSignature,
    SourceInterfaceSignatureFacts, SourceSymbolKey,
};
use skiff_syntax::{
    ast::InterfaceDecl,
    error::{CompileError, Result},
};

use crate::file_ir::{FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, TypeRefIr};

use super::{
    executable_type_projection::execution_type_ref, source_unit_lowering::source_span_ref,
};

pub(super) fn lower_interface_declaration(
    interface: &InterfaceDecl,
    interface_signatures: Option<&SourceInterfaceSignatureFacts>,
    module_path: &str,
) -> Result<InterfaceDeclIr> {
    let interface_signatures = interface_signatures.ok_or_else(|| {
        CompileError::Semantic(format!(
            "interface `{module_path}.{}` cannot lower without exact source requirement facts",
            interface.name
        ))
    })?;
    let interface_key = SourceSymbolKey::new(module_path, &interface.name);
    let exact_operation_count = interface_signatures
        .requirements()
        .filter(|(key, _)| key.interface == interface_key)
        .count();
    if exact_operation_count != interface.operations.len() {
        return Err(CompileError::Semantic(format!(
            "interface `{interface_key}` has {} source operations but {exact_operation_count} exact requirement facts",
            interface.operations.len()
        )));
    }
    let mut operation_names = BTreeSet::new();
    Ok(InterfaceDeclIr {
        name: interface.name.clone(),
        type_params: interface.type_params.clone(),
        // Syntax owns declaration order and source spans. Exact source facts
        // own every executable part of the operation signature below.
        operations: interface
            .operations
            .iter()
            .map(|operation| {
                if !operation_names.insert(operation.name.clone()) {
                    return Err(CompileError::Semantic(format!(
                        "interface `{interface_key}` declares operation `{}` more than once",
                        operation.name
                    )));
                }
                let key = SourceInterfaceMethodKey {
                    interface: interface_key.clone(),
                    method: operation.name.clone(),
                };
                let signature = interface_signatures.requirement(&key).ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "interface operation `{}.{}` has no exact source requirement fact",
                        interface_key, operation.name
                    ))
                })?;
                if signature.interface_type_params != interface.type_params {
                    return Err(CompileError::Semantic(format!(
                        "interface operation `{}.{}` exact type parameters do not match its declaration",
                        interface_key, operation.name
                    )));
                }
                lower_interface_operation(&operation.name, signature)
            })
            .collect::<Result<Vec<_>>>()?,
        source_span: Some(source_span_ref(interface.span)),
    })
}

fn lower_interface_operation(
    operation_name: &str,
    signature: &SourceInterfaceRequirementSignature,
) -> Result<InterfaceOperationIr> {
    let implicit_self = match &signature.receiver {
        SourceExecutableReceiver::None => {
            return Err(CompileError::Semantic(format!(
                "interface operation `{operation_name}` has no exact receiver"
            )));
        }
        SourceExecutableReceiver::Implicit { ty } => Some(interface_execution_type_ref(ty)),
        SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 } => {
            if !signature
                .parameters
                .first()
                .is_some_and(|parameter| parameter.name == "self")
            {
                return Err(CompileError::Semantic(format!(
                    "interface operation `{operation_name}` exact receiver points to parameter 0, but that parameter is not `self`"
                )));
            }
            None
        }
        SourceExecutableReceiver::ExplicitParameter { parameter_index } => {
            return Err(CompileError::Semantic(format!(
                "interface operation `{operation_name}` exact receiver points to unsupported parameter {parameter_index}"
            )));
        }
    };
    Ok(InterfaceOperationIr {
        name: operation_name.to_string(),
        type_params: signature.method_type_params.clone(),
        params: signature
            .parameters
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: interface_execution_type_ref(&param.ty),
            })
            .collect(),
        return_type: interface_execution_type_ref(&signature.return_type),
        is_native: signature.is_native,
        is_provider: signature.is_provider,
        is_static: signature.is_static,
        implicit_self,
    })
}

fn interface_execution_type_ref(ty: &PackageTypeRef) -> TypeRefIr {
    match execution_type_ref(ty) {
        TypeRefIr::TypeParam { name } if name == "Self" => TypeRefIr::builtin("Self"),
        ty => ty,
    }
}
