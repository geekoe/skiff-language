use crate::runtime_manifest_model::ArtifactOperation;
use skiff_artifact_model::{FileIrUnit, InterfaceDeclIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr};
use skiff_compiler_projection_input::ProjectionSourceMetadata;

use super::type_key::ContractTypeKey;
use crate::prelude::PreludeProjection;

use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContractProjection {
    pub prelude: PreludeProjection,
    pub types: BTreeMap<String, ContractTypeProjection>,
    pub aliases: BTreeMap<String, ContractAliasProjection>,
    pub interfaces: BTreeMap<String, ContractInterfaceProjection>,
    pub api_bindings: BTreeMap<String, ContractApiBindingProjection>,
    pub public_symbols_by_source: BTreeMap<String, String>,
    pub type_symbols: BTreeMap<String, String>,
    pub interface_symbols: BTreeMap<String, String>,
    pub interface_source_symbols: BTreeMap<String, String>,
}

impl ContractProjection {
    pub fn prelude(&self) -> &PreludeProjection {
        &self.prelude
    }

    pub fn has_exported_interfaces(&self) -> bool {
        !self.interfaces.is_empty()
    }

    pub fn split_operation_name<'a>(
        &'a self,
        operation_name: &'a str,
    ) -> Option<(&'a str, &'a str)> {
        self.interfaces
            .iter()
            .find_map(|(interface_name, interface)| {
                let method_name = operation_name.strip_prefix(&format!("{interface_name}."))?;
                interface
                    .operations
                    .iter()
                    .any(|operation| operation.name == method_name)
                    .then_some((interface_name.as_str(), method_name))
            })
    }

    pub fn operation(&self, operation_name: &str) -> Option<&ContractInterfaceOperationProjection> {
        let (interface_name, method_name) = self.split_operation_name(operation_name)?;
        self.interfaces
            .get(interface_name)?
            .operations
            .iter()
            .find(|operation| operation.name == method_name)
    }

    pub fn operation_binding(
        &self,
        interface_name: &str,
        method_name: &str,
    ) -> Option<&ContractOperationBindingProjection> {
        self.api_bindings
            .get(interface_name)?
            .operations
            .get(method_name)
    }

    pub fn operation_names(&self) -> std::collections::BTreeSet<String> {
        self.api_bindings
            .iter()
            .flat_map(|(interface_name, binding)| {
                binding
                    .operations
                    .keys()
                    .map(move |operation_name| format!("{interface_name}.{operation_name}"))
            })
            .collect()
    }

    pub fn artifact_operations(&self, service_target_component: &str) -> Vec<ArtifactOperation> {
        self.api_bindings
            .iter()
            .flat_map(|(interface_name, binding)| {
                binding
                    .operations
                    .iter()
                    .map(move |(method_name, operation_binding)| {
                        let operation_name = format!("{interface_name}.{method_name}");
                        ArtifactOperation {
                            operation: operation_name.clone(),
                            target: Some(format!(
                                "service.{service_target_component}.{operation_name}"
                            )),
                            function: operation_binding.executable_symbol.clone(),
                            parameters: self
                                .interfaces
                                .get(interface_name)
                                .and_then(|interface| {
                                    interface
                                        .operations
                                        .iter()
                                        .find(|operation| operation.name == *method_name)
                                })
                                .map(|operation| {
                                    operation
                                        .params
                                        .iter()
                                        .map(|parameter| parameter.name.clone())
                                        .collect()
                                })
                                .unwrap_or_default(),
                        }
                    })
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractTypeProjection {
    pub public_name: String,
    pub source_module: String,
    pub source_name: String,
    pub type_params: Vec<String>,
    pub descriptor: ContractTypeDescriptorProjection,
    pub discriminator: Option<String>,
    pub implements: Vec<ContractTypeKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractTypeDescriptorProjection {
    Record {
        fields: BTreeMap<String, ContractTypeKey>,
    },
    Union {
        variants: Vec<ContractTypeKey>,
    },
    Native {
        symbol: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractAliasProjection {
    pub public_name: String,
    pub source_module: String,
    pub source_name: String,
    pub type_params: Vec<String>,
    pub target: ContractTypeKey,
    pub discriminator: Option<String>,
    pub transparent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractInterfaceProjection {
    pub public_name: String,
    pub source_module: String,
    pub source_name: String,
    pub operations: Vec<ContractInterfaceOperationProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractInterfaceOperationProjection {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<ContractFunctionParamProjection>,
    pub return_type: ContractTypeKey,
    pub is_native: bool,
    pub is_provider: bool,
    pub is_static: bool,
    pub implicit_self: Option<ContractTypeKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractFunctionParamProjection {
    pub name: String,
    pub ty: ContractTypeKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractApiBindingProjection {
    pub source_module: String,
    pub source_symbol: String,
    pub operations: BTreeMap<String, ContractOperationBindingProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractOperationBindingProjection {
    pub module_path: String,
    pub type_name: String,
    pub method_name: String,
    pub executable_symbol: String,
    pub signature_module_path: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ContractProjectionUnit<'a> {
    pub unit: &'a FileIrUnit,
    pub source: &'a ProjectionSourceMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractTypeKind {
    Type,
    Alias,
    Interface,
}

#[derive(Clone, Copy, Debug)]
pub struct ContractProjectionTypeBinding<'a> {
    pub unit: &'a FileIrUnit,
    pub module_path: &'a str,
    pub local_name: &'a str,
    pub type_decl: &'a TypeDeclIr,
    pub interface: Option<&'a InterfaceDeclIr>,
    pub kind: ContractTypeKind,
    pub transparent_alias: bool,
}

impl<'a> ContractProjectionTypeBinding<'a> {
    pub fn new(
        unit: &'a FileIrUnit,
        local_name: &'a str,
        type_decl: &'a TypeDeclIr,
        interface: Option<&'a InterfaceDeclIr>,
    ) -> Self {
        let source_kind = declaration_source_kind(unit, local_name);
        let transparent_alias = !matches!(source_kind, Some("type"))
            && matches!(type_decl.descriptor, TypeDescriptorIr::Alias { .. });
        let kind = if interface.is_some() {
            ContractTypeKind::Interface
        } else if matches!(type_decl.descriptor, TypeDescriptorIr::Alias { .. }) {
            ContractTypeKind::Alias
        } else {
            ContractTypeKind::Type
        };
        Self {
            unit,
            module_path: unit.module_path.as_str(),
            local_name,
            type_decl,
            interface,
            kind,
            transparent_alias,
        }
    }

    pub fn descriptor(&self) -> &'a TypeDescriptorIr {
        &self.type_decl.descriptor
    }

    pub fn discriminator(&self) -> Option<&'a str> {
        self.type_decl.discriminator.as_deref()
    }

    pub fn implements(&self) -> &'a [TypeRefIr] {
        &self.type_decl.implements
    }
}

fn declaration_source_kind<'a>(unit: &'a FileIrUnit, local_name: &str) -> Option<&'a str> {
    unit.source_map
        .spans
        .iter()
        .find(|span| span.name.as_deref() == Some(local_name))
        .map(|span| span.kind.as_str())
}
