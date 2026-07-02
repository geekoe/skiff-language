use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::TypeRefIr;
use skiff_compiler_source::{
    api::PublicTypeKind, parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name, type_indices, LocalDbObjectIndex, ResolutionModels,
    SourceCompileModel, SourceIndexes,
};
use skiff_syntax::{
    ast::{AliasDecl, FunctionDecl, InterfaceOperation, SourceFile, TypeDecl, TypeRef},
    type_syntax::generic_parts,
};

use super::{
    type_lowering::{lower_type_ref, TypeLoweringContext},
    type_ref_ir_source_text_with_local_types, EntryFunctionSignature, EntryParamSpec,
    EntryTypeSpec, PackageAbiType, PackageAbiTypeDescriptor,
};

#[derive(Clone, Debug, Default)]
pub struct EntrypointAbiIndex {
    functions_by_module: BTreeMap<String, BTreeMap<String, EntryFunctionSignature>>,
}

#[derive(Debug, Clone)]
struct PackageAbiFunctionSignature {
    operation: InterfaceOperation,
    inherited_type_params: Vec<String>,
}

struct PublishedPackageCallable {
    source_module: String,
    source_symbol: String,
}

impl EntrypointAbiIndex {
    pub fn build(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        indexes: &SourceIndexes,
        resolutions: &ResolutionModels,
    ) -> Result<Self, String> {
        let mut functions_by_module = BTreeMap::new();
        for parsed in parsed_sources {
            let module_path = parsed.module_path();
            let source_type_indices = type_indices(parsed.ast());
            let local_db_objects = LocalDbObjectIndex::from_declarations(module_path, parsed.ast())
                .map_err(|error| {
                    format!(
                        "failed to build entrypoint ABI db attachment index for {}: {error}",
                        parsed.relative_path().display()
                    )
                })?;
            let signatures = service_entrypoint_abi_signatures(
                parsed.ast(),
                &EntrypointAbiSourceContext {
                    source_path: parsed.relative_path().display().to_string(),
                    type_indices: &source_type_indices,
                    local_db_objects: &local_db_objects,
                    package_aliases,
                    indexes,
                    source_alias_targets: resolutions.alias_targets_for_module(module_path),
                },
            )?;
            functions_by_module.insert(module_path.to_string(), signatures);
        }
        Ok(Self {
            functions_by_module,
        })
    }

    pub fn function_signature(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<EntryFunctionSignature> {
        self.functions_by_module
            .get(module_path)
            .and_then(|functions| functions.get(symbol))
            .cloned()
    }
}

pub fn package_entrypoint_function_signature(
    source_model: &SourceCompileModel,
    entrypoint_abi: &EntrypointAbiIndex,
    package_id: &str,
    symbol_path: &str,
) -> Result<Option<(String, String, EntryFunctionSignature)>, String> {
    let Some(callable) =
        package_publication_callable_for_symbol(source_model, package_id, symbol_path)
    else {
        return Ok(None);
    };
    let signature = entrypoint_abi
        .function_signature(&callable.source_module, &callable.source_symbol)
        .or_else(|| {
            let source = package_source_for_module(source_model, &callable.source_module)?;
            package_abi_function_signature(
                source_model,
                source.ast(),
                &callable.source_module,
                &callable.source_symbol,
            )
            .ok()
        })
        .ok_or_else(|| {
            format!(
                "function {} not found in package api module {}",
                callable.source_symbol, callable.source_module
            )
        })?;
    Ok(Some((
        callable.source_module,
        callable.source_symbol,
        signature,
    )))
}

pub fn package_public_schema_type_names_for_module(
    source_model: &SourceCompileModel,
    module_path: &str,
) -> Vec<String> {
    source_model
        .export_bindings()
        .public_schema_types()
        .values()
        .filter(|public_type| public_type.source_module == module_path)
        .filter_map(|public_type| match public_type.kind {
            PublicTypeKind::Type | PublicTypeKind::Alias => Some(public_type.source_symbol.clone()),
            PublicTypeKind::Interface => None,
        })
        .collect()
}

pub fn package_public_schema_abi_types_for_module(
    source_model: &SourceCompileModel,
    module_path: &str,
) -> Result<Vec<PackageAbiType>, String> {
    let source = package_source_for_module(source_model, module_path).ok_or_else(|| {
        format!(
            "api module {} not found in compiled package source model",
            module_path
        )
    })?;
    package_public_schema_type_names_for_module(source_model, module_path)
        .into_iter()
        .map(|name| {
            package_abi_type(source_model, source.ast(), module_path, &name)?.ok_or_else(|| {
                format!(
                    "public type {} not found in package api module {} source model",
                    name, module_path
                )
            })
        })
        .collect()
}

struct EntrypointAbiSourceContext<'a> {
    source_path: String,
    type_indices: &'a BTreeMap<String, u32>,
    local_db_objects: &'a LocalDbObjectIndex,
    package_aliases: &'a BTreeMap<String, Vec<String>>,
    indexes: &'a SourceIndexes,
    source_alias_targets: &'a BTreeMap<String, String>,
}

fn service_entrypoint_abi_signatures(
    ast: &SourceFile,
    context: &EntrypointAbiSourceContext<'_>,
) -> Result<BTreeMap<String, EntryFunctionSignature>, String> {
    let mut signatures = BTreeMap::new();
    for function in &ast.functions {
        insert_service_entrypoint_abi_signature(
            &mut signatures,
            &function.name,
            function,
            context,
        )?;
    }
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            let declaration_name =
                impl_method_declaration_name(&implementation.target, &method.name);
            insert_service_entrypoint_abi_signature(
                &mut signatures,
                &declaration_name,
                method,
                context,
            )?;
        }
    }
    Ok(signatures)
}

fn insert_service_entrypoint_abi_signature(
    signatures: &mut BTreeMap<String, EntryFunctionSignature>,
    declaration_name: &str,
    function: &FunctionDecl,
    context: &EntrypointAbiSourceContext<'_>,
) -> Result<(), String> {
    let signature = service_entrypoint_abi_signature(declaration_name, function, context)?;
    signatures
        .entry(declaration_name.to_string())
        .or_insert(signature);
    Ok(())
}

fn service_entrypoint_abi_signature(
    declaration_name: &str,
    function: &FunctionDecl,
    context: &EntrypointAbiSourceContext<'_>,
) -> Result<EntryFunctionSignature, String> {
    let type_params = function
        .type_params
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let params = function
        .params
        .iter()
        .map(|param| {
            Ok(EntryParamSpec {
                name: param.name.clone(),
                ty: entrypoint_abi_type_spec(&param.ty, &type_params, declaration_name, context)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let return_type = entrypoint_abi_type_spec(
        &function.return_type,
        &type_params,
        declaration_name,
        context,
    )?;
    Ok(EntryFunctionSignature {
        name: declaration_name.to_string(),
        params,
        return_type,
        local_type_names: local_type_names_from_type_indices(context.type_indices),
    })
}

fn entrypoint_abi_type_spec(
    ty: &TypeRef,
    type_params: &BTreeSet<String>,
    declaration_name: &str,
    context: &EntrypointAbiSourceContext<'_>,
) -> Result<EntryTypeSpec, String> {
    let ir = lower_type_ref(
        ty,
        context.type_indices,
        context.local_db_objects,
        context.indexes.publication_db_metadata_index(),
        context.package_aliases,
        context.indexes.publication_type_symbols(),
        context.source_alias_targets,
        TypeLoweringContext::value_with_type_params(type_params),
    )
    .map_err(|error| {
        format!(
            "service entrypoint ABI {} {declaration_name}: failed to lower type {}: {error}",
            context.source_path, ty.name
        )
    })?;
    let local_type_names = local_type_names_from_type_indices(context.type_indices);
    let name = type_ref_ir_source_text_with_local_types(&ir, &|type_index| {
        local_type_names.get(&type_index).cloned()
    });
    Ok(EntryTypeSpec {
        name,
        ir,
        local_type_names,
    })
}

fn package_abi_function_signature(
    source_model: &SourceCompileModel,
    ast: &SourceFile,
    module_path: &str,
    symbol: &str,
) -> Result<EntryFunctionSignature, String> {
    let Some(signature) = callable_signature(ast, module_path, symbol) else {
        return Err(format!(
            "function {symbol} not found in package api module {module_path}"
        ));
    };
    let type_indices = type_indices(ast);
    let local_type_names = local_type_names_from_type_indices(&type_indices);
    let local_db_objects = LocalDbObjectIndex::from_declarations(module_path, ast)
        .map_err(|error| error.to_string())?;
    let type_params = signature
        .inherited_type_params
        .iter()
        .chain(signature.operation.type_params.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let context = TypeLoweringContext::value_with_type_params(&type_params);
    let lower = |ty: &TypeRef| {
        lower_type_ref(
            ty,
            &type_indices,
            &local_db_objects,
            source_model.indexes().publication_db_metadata_index(),
            source_model.dependencies().package_aliases(),
            source_model.indexes().publication_type_symbols(),
            source_model
                .resolutions()
                .alias_targets_for_module(module_path),
            context,
        )
        .map_err(|error| error.to_string())
    };

    Ok(EntryFunctionSignature {
        name: signature.operation.name.clone(),
        params: signature
            .operation
            .params
            .iter()
            .map(|param| {
                let ir = lower(&param.ty)?;
                Ok(EntryParamSpec {
                    name: param.name.clone(),
                    ty: EntryTypeSpec {
                        name: type_ref_ir_source_text_with_local_types(&ir, &|type_index| {
                            local_type_names.get(&type_index).cloned()
                        }),
                        ir,
                        local_type_names: local_type_names.clone(),
                    },
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        return_type: {
            let ir = lower(&signature.operation.return_type)?;
            EntryTypeSpec {
                name: type_ref_ir_source_text_with_local_types(&ir, &|type_index| {
                    local_type_names.get(&type_index).cloned()
                }),
                ir,
                local_type_names: local_type_names.clone(),
            }
        },
        local_type_names,
    })
}

fn package_source_for_module<'a>(
    source_model: &'a SourceCompileModel,
    module_path: &str,
) -> Option<&'a ParsedCompilerSource> {
    source_model
        .sources()
        .parsed_sources()
        .iter()
        .find(|source| source.module_path() == module_path)
}

fn package_public_path(package_id: &str, export_path: &str) -> String {
    if export_path.is_empty() {
        package_id.to_string()
    } else if package_id.is_empty() {
        export_path.to_string()
    } else {
        format!("{package_id}.{export_path}")
    }
}

fn package_abi_type(
    source_model: &SourceCompileModel,
    ast: &SourceFile,
    module_path: &str,
    name: &str,
) -> Result<Option<PackageAbiType>, String> {
    if let Some(ty) = ast.types.iter().find(|ty| ty.name == name) {
        return package_abi_type_from_decl(source_model, ast, module_path, ty).map(Some);
    }
    if let Some(alias) = ast.aliases.iter().find(|alias| alias.name == name) {
        return package_abi_type_from_alias(source_model, ast, module_path, alias).map(Some);
    }
    if ast
        .interfaces
        .iter()
        .any(|interface| interface.name == name)
    {
        return Ok(Some(PackageAbiType {
            name: name.to_string(),
            descriptor: PackageAbiTypeDescriptor::External,
            discriminator: None,
            local_type_names: local_type_names_from_type_indices(&type_indices(ast)),
        }));
    }
    Ok(None)
}

fn package_abi_type_from_decl(
    source_model: &SourceCompileModel,
    ast: &SourceFile,
    module_path: &str,
    ty: &TypeDecl,
) -> Result<PackageAbiType, String> {
    let type_indices = type_indices(ast);
    let local_db_objects = LocalDbObjectIndex::from_declarations(module_path, ast)
        .map_err(|error| error.to_string())?;
    let type_params = ty.type_params.iter().cloned().collect::<BTreeSet<_>>();
    let context = TypeLoweringContext::value_with_type_params(&type_params);
    let lower = |ty: &TypeRef| {
        lower_type_ref(
            ty,
            &type_indices,
            &local_db_objects,
            source_model.indexes().publication_db_metadata_index(),
            source_model.dependencies().package_aliases(),
            source_model.indexes().publication_type_symbols(),
            source_model
                .resolutions()
                .alias_targets_for_module(module_path),
            context,
        )
        .map_err(|error| error.to_string())
    };
    let descriptor = if let Some(alias) = &ty.alias {
        match lower(alias)? {
            TypeRefIr::Union { items } => PackageAbiTypeDescriptor::Union { variants: items },
            target => PackageAbiTypeDescriptor::Alias { target },
        }
    } else {
        PackageAbiTypeDescriptor::Record {
            fields: ty
                .fields
                .iter()
                .map(|field| Ok((field.name.clone(), lower(&field.ty)?)))
                .collect::<Result<BTreeMap<_, _>, String>>()?,
        }
    };
    Ok(PackageAbiType {
        name: ty.name.clone(),
        descriptor,
        discriminator: ty.discriminator.clone(),
        local_type_names: local_type_names_from_type_indices(&type_indices),
    })
}

fn package_abi_type_from_alias(
    source_model: &SourceCompileModel,
    ast: &SourceFile,
    module_path: &str,
    alias: &AliasDecl,
) -> Result<PackageAbiType, String> {
    let type_indices = type_indices(ast);
    let local_db_objects = LocalDbObjectIndex::from_declarations(module_path, ast)
        .map_err(|error| error.to_string())?;
    let target = lower_type_ref(
        &alias.target_type,
        &type_indices,
        &local_db_objects,
        source_model.indexes().publication_db_metadata_index(),
        source_model.dependencies().package_aliases(),
        source_model.indexes().publication_type_symbols(),
        source_model
            .resolutions()
            .alias_targets_for_module(module_path),
        TypeLoweringContext::value(),
    )
    .map_err(|error| error.to_string())?;
    Ok(PackageAbiType {
        name: alias.name.clone(),
        descriptor: PackageAbiTypeDescriptor::Alias { target },
        discriminator: None,
        local_type_names: local_type_names_from_type_indices(&type_indices),
    })
}

fn package_publication_callable_for_symbol(
    source_model: &SourceCompileModel,
    package_id: &str,
    symbol_path: &str,
) -> Option<PublishedPackageCallable> {
    source_model
        .export_bindings()
        .public_callables()
        .values()
        .find_map(|callable| {
            package_handler_symbol_matches_public_callable(
                package_id,
                &callable.public_path,
                symbol_path,
            )
            .then(|| PublishedPackageCallable {
                source_module: callable.source_module.clone(),
                source_symbol: callable.source_symbol.clone(),
            })
        })
}

fn package_handler_symbol_matches_public_callable(
    package_id: &str,
    public_path: &str,
    symbol_path: &str,
) -> bool {
    if symbol_path == public_path {
        return true;
    }
    let Some((export_path, symbol)) = public_path.rsplit_once('.') else {
        return false;
    };
    symbol_path == format!("{}.{symbol}", package_public_path(package_id, export_path))
}

fn callable_signature(
    ast: &SourceFile,
    module_path: &str,
    symbol: &str,
) -> Option<PackageAbiFunctionSignature> {
    ast.function_signatures
        .iter()
        .find(|function| function.name == symbol)
        .cloned()
        .map(|operation| PackageAbiFunctionSignature {
            operation,
            inherited_type_params: Vec::new(),
        })
        .or_else(|| {
            ast.functions
                .iter()
                .find(|function| function.name == symbol)
                .map(function_decl_signature)
        })
        .or_else(|| {
            let (target, method_name) = symbol.rsplit_once('.')?;
            ast.impls
                .iter()
                .filter(|implementation| {
                    callable_impl_target_matches(&implementation.target, module_path, target)
                })
                .find_map(|implementation| {
                    implementation
                        .methods
                        .iter()
                        .find(|method| method.name == method_name)
                        .cloned()
                        .map(|operation| PackageAbiFunctionSignature {
                            operation,
                            inherited_type_params: generic_type_params_from_text(
                                &implementation.target,
                            ),
                        })
                })
        })
}

fn callable_impl_target_matches(target: &str, module_path: &str, local_target: &str) -> bool {
    let target = target.strip_prefix("root.").unwrap_or(target);
    target == local_target || target == format!("{module_path}.{local_target}")
}

fn function_decl_signature(function: &FunctionDecl) -> PackageAbiFunctionSignature {
    PackageAbiFunctionSignature {
        operation: InterfaceOperation {
            name: function.name.clone(),
            type_params: function.type_params.clone(),
            params: function.params.clone(),
            return_type: function.return_type.clone(),
            is_native: function.is_native,
            is_provider: function.is_provider,
            is_static: function.is_static,
            implicit_self: function.implicit_self.clone(),
            span: function.span.clone(),
        },
        inherited_type_params: Vec::new(),
    }
}

fn generic_type_params_from_text(name: &str) -> Vec<String> {
    generic_parts(name)
        .map(|parts| {
            parts
                .args
                .iter()
                .map(|arg| arg.trim())
                .filter(|arg| {
                    !arg.is_empty()
                        && !is_builtin_wrapper_visible_type(arg)
                        && arg
                            .chars()
                            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn local_type_names_from_type_indices(
    type_indices: &BTreeMap<String, u32>,
) -> BTreeMap<u32, String> {
    type_indices
        .iter()
        .map(|(name, index)| (*index, name.clone()))
        .collect()
}

fn is_builtin_wrapper_visible_type(name: &str) -> bool {
    skiff_compiler_source::prelude_registry::is_builtin_type_name(name)
}
