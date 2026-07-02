use std::collections::BTreeMap;

use skiff_artifact_model::{AbiAliasId, AbiInterfaceId, AbiTypeId};
use skiff_artifact_model::{ExecutableIr, FileIrUnit, InterfaceDeclIr, TypeDeclIr, TypeRefIr};
use skiff_compiler_projection_input::{
    ExportCallableProjection, ExportSchemaProjection, ProjectionDeclarationKey,
    ProjectionSourceDeclarationKind, ProjectionSourceMetadata, ProjectionSourceSymbolKey,
    ProjectionView, PublicModuleExportProjection,
};

use crate::prelude::PreludeProjection;

use super::model::{ContractProjectionTypeBinding, ContractProjectionUnit};

pub struct ContractProjectionIndex<'a> {
    input: ProjectionView<'a>,
    units_by_module: BTreeMap<&'a str, &'a FileIrUnit>,
    sources_by_module: BTreeMap<&'a str, &'a ProjectionSourceMetadata>,
    prelude: Option<&'a PreludeProjection>,
}

impl<'a> ContractProjectionIndex<'a> {
    pub fn from_projection_input(input: ProjectionView<'a>) -> Self {
        Self::from_projection_input_with_prelude(input, None)
    }

    pub fn from_projection_input_with_prelude(
        input: ProjectionView<'a>,
        prelude: Option<&'a PreludeProjection>,
    ) -> Self {
        let units_by_module = input
            .file_ir_units()
            .iter()
            .map(|unit| (unit.module_path.as_str(), unit))
            .collect();
        let sources_by_module = input
            .source_metadata()
            .iter()
            .map(|source| (source.module_path.as_str(), source))
            .collect();
        Self {
            input,
            units_by_module,
            sources_by_module,
            prelude,
        }
    }

    pub fn prelude(&self) -> Option<&'a PreludeProjection> {
        self.prelude
    }

    pub fn unit_by_module_path(&self, module_path: &str) -> Option<&'a FileIrUnit> {
        self.units_by_module.get(module_path).copied()
    }

    pub fn source_metadata_by_module_path(
        &self,
        module_path: &str,
    ) -> Option<&'a ProjectionSourceMetadata> {
        self.sources_by_module.get(module_path).copied()
    }

    pub fn contract_facing_modules(&self) -> impl Iterator<Item = ContractProjectionUnit<'a>> + '_ {
        self.input.source_metadata().iter().filter_map(|source| {
            source
                .role
                .is_contract()
                .then(|| {
                    self.unit_by_module_path(&source.module_path)
                        .map(|unit| (unit, source))
                })
                .flatten()
                .map(|(unit, source)| ContractProjectionUnit { unit, source })
        })
    }

    pub fn implementation_units(&self) -> impl Iterator<Item = &'a FileIrUnit> + '_ {
        self.input.file_ir_units().iter()
    }

    pub fn implementation_modules(&self) -> impl Iterator<Item = ContractProjectionUnit<'a>> + '_ {
        self.input.file_ir_units().iter().filter_map(|unit| {
            self.source_metadata_by_module_path(&unit.module_path)
                .map(|source| ContractProjectionUnit { unit, source })
        })
    }

    pub fn type_decl_by_module_local_name(
        &self,
        module_path: &str,
        local_name: &str,
    ) -> Option<&'a TypeDeclIr> {
        let unit = self.unit_by_module_path(module_path)?;
        let declaration = unit.declarations.types.get(local_name)?;
        unit.type_table.get(declaration.type_index as usize)
    }

    pub fn interface_decl_by_module_local_name(
        &self,
        module_path: &str,
        local_name: &str,
    ) -> Option<&'a InterfaceDeclIr> {
        self.unit_by_module_path(module_path)?
            .declarations
            .interfaces
            .get(local_name)
    }

    pub fn type_binding_by_module_local_name(
        &self,
        module_path: &str,
        local_name: &str,
    ) -> Option<ContractProjectionTypeBinding<'a>> {
        let unit = self.unit_by_module_path(module_path)?;
        let (local_name, declaration) = unit.declarations.types.get_key_value(local_name)?;
        let type_decl = unit.type_table.get(declaration.type_index as usize)?;
        let interface = unit.declarations.interfaces.get(local_name);
        Some(ContractProjectionTypeBinding::new(
            unit,
            local_name.as_str(),
            type_decl,
            interface,
        ))
    }

    pub fn type_binding_by_module_type_index(
        &self,
        module_path: &str,
        type_index: u32,
    ) -> Option<ContractProjectionTypeBinding<'a>> {
        let unit = self.unit_by_module_path(module_path)?;
        let type_decl = unit.type_table.get(type_index as usize)?;
        let (local_name, declaration) = unit.declarations.types.get_key_value(&type_decl.name)?;
        (declaration.type_index == type_index).then_some(())?;
        let interface = unit.declarations.interfaces.get(local_name);
        Some(ContractProjectionTypeBinding::new(
            unit,
            local_name.as_str(),
            type_decl,
            interface,
        ))
    }

    pub fn type_binding_by_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
    ) -> Option<ContractProjectionTypeBinding<'a>> {
        self.type_binding_by_module_local_name(source_key.module_path(), source_key.symbol())
    }

    pub fn local_type_ref_by_module_local_name(
        &self,
        module_path: &str,
        local_name: &str,
    ) -> Option<TypeRefIr> {
        let unit = self.unit_by_module_path(module_path)?;
        let declaration = unit.declarations.types.get(local_name)?;
        Some(TypeRefIr::LocalType {
            type_index: declaration.type_index,
        })
    }

    pub fn executable_by_module_symbol(
        &self,
        module_path: &str,
        executable_symbol: &str,
    ) -> Option<&'a ExecutableIr> {
        let unit = self.unit_by_module_path(module_path)?;
        let declaration = unit.declarations.executables.get(executable_symbol)?;
        unit.executables.get(declaration.executable_index as usize)
    }

    pub fn module_exports(&self) -> &'a [PublicModuleExportProjection] {
        self.input.source().export_bindings().module_exports()
    }

    pub fn public_modules(&self) -> &'a BTreeMap<String, String> {
        &self.input.source().publication_api_seed().public_modules
    }

    pub fn public_schema_bindings(&self) -> impl Iterator<Item = &'a ExportSchemaProjection> + '_ {
        self.input
            .source()
            .export_bindings()
            .public_schema_types()
            .values()
    }

    pub fn public_callable_bindings(
        &self,
    ) -> impl Iterator<Item = &'a ExportCallableProjection> + '_ {
        self.input
            .source()
            .export_bindings()
            .public_callables()
            .values()
    }

    pub fn public_symbol_for_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
    ) -> Option<String> {
        self.input
            .source()
            .export_bindings()
            .public_schema_types()
            .values()
            .find(|schema| {
                source_key.module_path() == schema.source_module
                    && source_key.symbol() == schema.source_symbol
            })
            .map(|schema| schema.public_path.clone())
            .or_else(|| {
                self.input
                    .source()
                    .export_bindings()
                    .public_symbols()
                    .values()
                    .find(|symbol| {
                        source_key.module_path() == symbol.source_module
                            && source_key.symbol() == symbol.source_symbol
                    })
                    .map(|symbol| symbol.public_path.clone())
            })
            .or_else(|| {
                self.input
                    .source()
                    .export_bindings()
                    .public_callables()
                    .values()
                    .find(|callable| {
                        source_key.module_path() == callable.source_module
                            && source_key.symbol() == callable.source_symbol
                    })
                    .map(|callable| callable.public_path.clone())
            })
    }

    pub fn source_key_for_reference_symbol(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<ProjectionSourceSymbolKey> {
        let public_path = if module_path.is_empty() {
            symbol.to_string()
        } else {
            format!("{module_path}.{symbol}")
        };
        self.input
            .source()
            .export_bindings()
            .public_symbols()
            .get(&public_path)
            .map(|binding| {
                ProjectionSourceSymbolKey::new(&binding.source_module, &binding.source_symbol)
            })
            .or_else(|| {
                self.input
                    .source()
                    .export_bindings()
                    .public_schema_types()
                    .get(&public_path)
                    .map(|binding| {
                        ProjectionSourceSymbolKey::new(
                            &binding.source_module,
                            &binding.source_symbol,
                        )
                    })
            })
            .or_else(|| {
                self.input
                    .source()
                    .export_bindings()
                    .public_callables()
                    .get(&public_path)
                    .map(|binding| {
                        ProjectionSourceSymbolKey::new(
                            &binding.source_module,
                            &binding.source_symbol,
                        )
                    })
            })
    }

    pub fn is_public_schema_source_key(&self, source_key: &ProjectionSourceSymbolKey) -> bool {
        self.input
            .source()
            .export_bindings()
            .public_schema_types()
            .values()
            .any(|schema| {
                source_key.module_path() == schema.source_module
                    && source_key.symbol() == schema.source_symbol
            })
    }

    pub fn is_public_callable_source_key(&self, source_key: &ProjectionSourceSymbolKey) -> bool {
        self.input
            .source()
            .export_bindings()
            .public_callables()
            .values()
            .any(|callable| {
                source_key.module_path() == callable.source_module
                    && source_key.symbol() == callable.source_symbol
            })
    }

    pub fn source_module_for_reference_module<'s>(&'s self, module_path: &'s str) -> &'s str {
        self.input
            .source()
            .export_bindings()
            .public_symbols()
            .values()
            .find_map(|symbol| {
                (public_module_path_from_public_symbol(&symbol.public_path) == module_path)
                    .then_some(symbol.source_module.as_str())
            })
            .unwrap_or(module_path)
    }
}

fn public_module_path_from_public_symbol(public_path: &str) -> &str {
    public_path
        .rsplit_once('.')
        .map(|(module, _)| module)
        .unwrap_or("")
}

impl ContractProjectionIndex<'_> {
    pub fn abi_type_id_for_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
        kind: ProjectionSourceDeclarationKind,
    ) -> Option<AbiTypeId> {
        self.input
            .source()
            .abi_ids()
            .get(&ProjectionDeclarationKey::new(source_key, kind))
            .and_then(|ids| ids.type_id.clone())
    }

    pub fn abi_alias_id_for_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
        kind: ProjectionSourceDeclarationKind,
    ) -> Option<AbiAliasId> {
        self.input
            .source()
            .abi_ids()
            .get(&ProjectionDeclarationKey::new(source_key, kind))
            .and_then(|ids| ids.alias_id.clone())
    }

    pub fn abi_interface_id_for_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
        kind: ProjectionSourceDeclarationKind,
    ) -> Option<AbiInterfaceId> {
        self.input
            .source()
            .abi_ids()
            .get(&ProjectionDeclarationKey::new(source_key, kind))
            .and_then(|ids| ids.interface_id.clone())
    }

    /// 通过 public path 反查 `ProjectionSourceSymbolKey`,供 `abi_type_id_for_named_key` 使用。
    ///
    /// 这里 public path 形如 `"public.Event"` 或 `"admin.Input"`,需要拆 module 部分来查。
    pub fn source_key_for_named_key_public(
        &self,
        public_symbol: &str,
    ) -> Option<ProjectionSourceSymbolKey> {
        // Try direct lookup first (public_symbol is the full public_path key).
        if let Some(key) = self
            .input
            .source()
            .export_bindings()
            .public_schema_types()
            .get(public_symbol)
            .map(|b| ProjectionSourceSymbolKey::new(&b.source_module, &b.source_symbol))
        {
            return Some(key);
        }
        // Also try through general public symbols.
        if let Some(key) = self
            .input
            .source()
            .export_bindings()
            .public_symbols()
            .get(public_symbol)
            .map(|b| ProjectionSourceSymbolKey::new(&b.source_module, &b.source_symbol))
        {
            return Some(key);
        }
        None
    }
}
