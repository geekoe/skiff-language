use std::collections::{BTreeMap, BTreeSet};

use crate::file_ir::{FileIrUnit, TypeRefIr};
use skiff_compiler_source::{
    api::PublicTypeKind, parsed_sources::ParsedCompilerSource, type_indices, LocalDbObjectIndex,
    PackageSourceModel,
};
use skiff_syntax::ast::{AliasDecl, SourceFile, TypeDecl, TypeRef};

use super::{
    type_lowering::{lower_type_ref, TypeLoweringContext},
    type_ref_ir_source_text_with_local_types, EntryFunctionSignature, EntryParamSpec,
    EntryTypeSpec, PackageAbiType, PackageAbiTypeDescriptor,
};

#[derive(Clone, Debug, Default)]
pub struct EntrypointAbiIndex {
    functions_by_module: BTreeMap<String, BTreeMap<String, EntryFunctionSignature>>,
}

struct PublishedPackageCallable {
    source_module: String,
    source_symbol: String,
}

impl EntrypointAbiIndex {
    pub fn build(file_ir_units: &[FileIrUnit]) -> Result<Self, String> {
        let mut functions_by_module = BTreeMap::new();
        for unit in file_ir_units {
            let local_type_names = unit
                .declarations
                .types
                .iter()
                .map(|(name, declaration)| (declaration.type_index, name.clone()))
                .collect::<BTreeMap<_, _>>();
            let signatures = unit
                .declarations
                .executables
                .iter()
                .map(|(name, declaration)| {
                    let executable = unit
                        .executables
                        .get(declaration.executable_index as usize)
                        .ok_or_else(|| {
                            format!(
                                "File IR executable `{}` in module `{}` points outside the executable table",
                                name, unit.module_path
                            )
                        })?;
                    let params = executable
                        .params
                        .iter()
                        .map(|parameter| EntryParamSpec {
                            name: parameter.name.clone(),
                            ty: file_ir_entry_type_spec(&parameter.ty, &local_type_names),
                        })
                        .collect();
                    Ok((
                        name.clone(),
                        EntryFunctionSignature {
                            name: name.clone(),
                            params,
                            return_type: file_ir_entry_type_spec(
                                &executable.return_type,
                                &local_type_names,
                            ),
                            local_type_names: local_type_names.clone(),
                        },
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?;
            if functions_by_module
                .insert(unit.module_path.clone(), signatures)
                .is_some()
            {
                return Err(format!(
                    "more than one File IR unit declares module `{}`",
                    unit.module_path
                ));
            }
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

fn file_ir_entry_type_spec(
    ty: &TypeRefIr,
    local_type_names: &BTreeMap<u32, String>,
) -> EntryTypeSpec {
    EntryTypeSpec {
        name: type_ref_ir_source_text_with_local_types(ty, &|type_index| {
            local_type_names.get(&type_index).cloned()
        }),
        ir: ty.clone(),
        local_type_names: local_type_names.clone(),
    }
}

pub fn package_entrypoint_function_signature(
    source_model: &PackageSourceModel,
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
    source_model: &PackageSourceModel,
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
    source_model: &PackageSourceModel,
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

fn package_source_for_module<'a>(
    source_model: &'a PackageSourceModel,
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
    source_model: &PackageSourceModel,
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
    source_model: &PackageSourceModel,
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
    source_model: &PackageSourceModel,
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
    source_model: &PackageSourceModel,
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

fn local_type_names_from_type_indices(
    type_indices: &BTreeMap<String, u32>,
) -> BTreeMap<u32, String> {
    type_indices
        .iter()
        .map(|(name, index)| (*index, name.clone()))
        .collect()
}
