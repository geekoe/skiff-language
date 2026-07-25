use std::collections::BTreeMap;

use crate::file_ir::{FileIrUnit, TypeRefIr};
use skiff_compiler_source::{api::PublicTypeKind, PackageSourceModel};

use super::{
    type_ref_ir_source_text_with_local_types, EntryFunctionSignature, EntryParamSpec,
    EntryTypeSpec, PackageAbiType,
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
            PublicTypeKind::Type | PublicTypeKind::Alias | PublicTypeKind::Interface => {
                Some(public_type.source_symbol.clone())
            }
        })
        .collect()
}

pub fn package_public_schema_abi_types_for_module(
    source_model: &PackageSourceModel,
    file_ir_units: &[FileIrUnit],
    module_path: &str,
) -> Result<Vec<PackageAbiType>, String> {
    let mut matching_units = file_ir_units
        .iter()
        .filter(|unit| unit.module_path == module_path);
    let unit = matching_units
        .next()
        .ok_or_else(|| format!("api module {module_path} has no canonical File IR unit"))?;
    if matching_units.next().is_some() {
        return Err(format!(
            "api module {module_path} has more than one canonical File IR unit"
        ));
    }
    package_public_schema_type_names_for_module(source_model, module_path)
        .into_iter()
        .map(|name| {
            let declaration = unit.declarations.types.get(&name).ok_or_else(|| {
                format!(
                    "public type {name} has no canonical declaration in File IR module {module_path}"
                )
            })?;
            let ty = unit
                .type_table
                .get(declaration.type_index as usize)
                .ok_or_else(|| {
                    format!(
                        "public type {name} points outside the canonical File IR type table at {}",
                        declaration.type_index
                    )
                })?;
            if ty.name != name {
                return Err(format!(
                    "public type {name} resolves to mismatched canonical File IR declaration {}",
                    ty.name
                ));
            }
            Ok(ty.clone())
        })
        .collect()
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
