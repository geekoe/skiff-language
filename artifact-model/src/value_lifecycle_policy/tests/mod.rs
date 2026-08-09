use super::*;
use crate::{
    ContractTypeDescriptor, ContractTypeRef, PackageSchemaCanonicalDescriptor, PackageSchemaTypeId,
    TypeRefIr,
};

mod descriptors;
mod identity;
mod plans;

pub(super) struct RejectingResolver;

impl ValueLifecycleFactResolver for RejectingResolver {
    fn resolve_package_symbol(
        &mut self,
        _symbol: &crate::PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        Err(resolver_error())
    }

    fn resolve_package_schema(
        &mut self,
        _package_id: &str,
        _stable_schema_key: &str,
        _package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<crate::PackageSchemaTypeRecord, ValueLifecycleResolverError> {
        Err(resolver_error())
    }

    fn validate_interface(
        &mut self,
        _interface: &crate::InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        Err(resolver_error())
    }

    fn validate_contract_interface(
        &mut self,
        _interface: &ContractTypeRef,
        _arguments: &[ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError> {
        Err(resolver_error())
    }
}

pub(super) struct PackageResolver {
    pub(super) calls: usize,
    pub(super) descriptor: PackageDescriptor,
}

#[derive(Clone, Copy)]
pub(super) enum PackageDescriptor {
    AliasString,
    Cycle,
    RepresentationStream,
}

impl ValueLifecycleFactResolver for PackageResolver {
    fn resolve_package_symbol(
        &mut self,
        symbol: &crate::PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        self.calls += 1;
        Ok(ResolvedPackageValueType {
            type_parameters: Vec::new(),
            descriptor: match self.descriptor {
                PackageDescriptor::AliasString => crate::TypeDescriptorIr::Alias {
                    target: TypeRefIr::builtin("string"),
                },
                PackageDescriptor::Cycle => crate::TypeDescriptorIr::Alias {
                    target: TypeRefIr::PackageSymbol {
                        symbol: symbol.clone(),
                    },
                },
                PackageDescriptor::RepresentationStream => {
                    crate::TypeDescriptorIr::Representation {
                        representation: TypeRefIr::Builtin {
                            name: "Stream".to_string(),
                            args: vec![TypeRefIr::builtin("string")],
                        },
                    }
                }
            },
        })
    }

    fn resolve_package_schema(
        &mut self,
        _package_id: &str,
        _stable_schema_key: &str,
        _package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<crate::PackageSchemaTypeRecord, ValueLifecycleResolverError> {
        Err(resolver_error())
    }

    fn validate_interface(
        &mut self,
        _interface: &crate::InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        Err(resolver_error())
    }

    fn validate_contract_interface(
        &mut self,
        _interface: &ContractTypeRef,
        _arguments: &[ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError> {
        Err(resolver_error())
    }
}

#[derive(Clone, Copy)]
pub(super) enum SchemaMode {
    Cycle,
    Callback,
    AnyInterface { argument_count: usize },
    Enumeration,
}

pub(super) struct SchemaResolver {
    pub(super) mode: SchemaMode,
}

impl ValueLifecycleFactResolver for SchemaResolver {
    fn resolve_package_symbol(
        &mut self,
        _symbol: &crate::PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        Err(resolver_error())
    }

    fn resolve_package_schema(
        &mut self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<crate::PackageSchemaTypeRecord, ValueLifecycleResolverError> {
        let descriptor = match self.mode {
            SchemaMode::Cycle => ContractTypeDescriptor::Alias {
                target: ContractTypeRef::PackageSchema {
                    package_id: package_id.to_string(),
                    stable_schema_key: stable_schema_key.to_string(),
                    package_schema_type_id: package_schema_type_id.clone(),
                },
            },
            SchemaMode::Callback => ContractTypeDescriptor::CallbackInterface {
                operations: Default::default(),
            },
            SchemaMode::AnyInterface { argument_count } => ContractTypeDescriptor::Alias {
                target: ContractTypeRef::AnyInterface {
                    interface: Box::new(ContractTypeRef::PackageSchema {
                        package_id: package_id.to_string(),
                        stable_schema_key: "callback".to_string(),
                        package_schema_type_id: PackageSchemaTypeId::new("callback-id"),
                    }),
                    arguments: (0..argument_count)
                        .map(|_| ContractTypeRef::builtin("string"))
                        .collect(),
                },
            },
            SchemaMode::Enumeration => ContractTypeDescriptor::Enumeration {
                variants: vec!["first".to_string()],
            },
        };
        Ok(crate::PackageSchemaTypeRecord {
            package_id: package_id.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: package_schema_type_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor,
            },
        })
    }

    fn validate_interface(
        &mut self,
        _interface: &crate::InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        Err(resolver_error())
    }

    fn validate_contract_interface(
        &mut self,
        interface: &ContractTypeRef,
        arguments: &[ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError> {
        let exact_target = matches!(
            interface,
            ContractTypeRef::PackageSchema { stable_schema_key, .. }
                if stable_schema_key == "callback"
        );
        if exact_target && arguments.len() == 1 {
            Ok(())
        } else {
            Err(ValueLifecycleResolverError {
                authority: "test.callback".to_string(),
                message: "target is not the exact one-parameter CallbackInterface".to_string(),
            })
        }
    }
}

pub(super) fn resolver_error() -> ValueLifecycleResolverError {
    ValueLifecycleResolverError {
        authority: "test".to_string(),
        message: "not present".to_string(),
    }
}

pub(super) fn budget() -> ValueLifecyclePolicyBudget {
    ValueLifecyclePolicyBudget::new(1_000, 1_000_000, 64).unwrap()
}
