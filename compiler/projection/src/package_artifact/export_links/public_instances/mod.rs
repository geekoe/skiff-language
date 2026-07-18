mod interfaces;
mod operations;

#[cfg(test)]
pub(in crate::package_artifact) use operations::package_public_instance_method_operation;

use std::collections::BTreeMap;

use skiff_artifact_identity::type_ref_abi_key;
use skiff_artifact_model::{
    FileIrRef, FileIrUnit, InterfaceInstantiationRef, InterfaceMethodSignature,
    OperationConstReceiverRef, PackageExportIndex, PublicInstanceExport, ServiceSymbolRef,
    TypeRefIr,
};

use skiff_compiler_core::package_interface_methods::PackageTypeSymbolIndex;

use crate::{error::ProjectionError, package_artifact::visible_types::PackageVisibleTypeNames};

use super::package_scoped_export_symbol;
use crate::package_artifact::model::PackageExportLinkProjectionInput;

pub(super) fn project_package_public_instances(
    package: &PackageExportLinkProjectionInput<'_>,
    files_by_module: &BTreeMap<String, FileIrRef>,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    type_symbols: &PackageTypeSymbolIndex,
    package_type_names: &PackageVisibleTypeNames,
    exports: &mut PackageExportIndex,
) -> Result<(), ProjectionError> {
    let mut seen_instances = BTreeMap::<String, String>::new();
    for public_instance in &package.exports.public_instances {
        let public_path = package_scoped_export_symbol(package, &public_instance.public_path);
        let source = format!(
            "{}.{}",
            public_instance.module, public_instance.const_symbol
        );
        if let Some(existing) = seen_instances.insert(public_path.clone(), source.clone()) {
            return Err(public_instance_error(
                package,
                &public_path,
                format!("duplicate public instance exported by both {existing} and {source}"),
            ));
        }
        let receiver_unit = file_units_by_module
            .get(public_instance.module.as_str())
            .copied()
            .ok_or_else(|| {
                public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector points to missing module {}",
                        public_instance.module
                    ),
                )
            })?;
        let receiver_file = files_by_module
            .get(&public_instance.module)
            .cloned()
            .ok_or_else(|| {
                public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector points to missing File IR ref for module {}",
                        public_instance.module
                    ),
                )
            })?;
        let const_decl = receiver_unit
            .declarations
            .constants
            .get(&public_instance.const_symbol)
            .ok_or_else(|| {
                public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector points to missing const {}.{}",
                        public_instance.module, public_instance.const_symbol
                    ),
                )
            })?;
        let constant = receiver_unit
            .constants
            .get(const_decl.const_index as usize)
            .ok_or_else(|| {
                public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector {}.{} points to missing const index {}",
                        public_instance.module,
                        public_instance.const_symbol,
                        const_decl.const_index
                    ),
                )
            })?;
        let receiver_const = OperationConstReceiverRef {
            file_ref: receiver_file,
            const_index: const_decl.const_index,
            const_abi_id: format!(
                "const:{}.{}",
                public_instance.module, public_instance.const_symbol
            ),
            const_type_abi_id: type_ref_abi_key(&constant.ty),
        };
        let receiver = interfaces::resolve_receiver(
            package,
            file_units_by_module,
            &public_path,
            &public_instance.module,
            &constant.ty,
        )?;
        let implemented_interfaces = interfaces::resolve_interfaces(
            package,
            file_units_by_module,
            type_symbols,
            package_type_names,
            &public_path,
            &receiver,
            &public_instance.interfaces,
        )?;
        let instance_operations = operations::project_operations(
            package,
            files_by_module,
            &public_path,
            &receiver,
            &receiver_const,
            &implemented_interfaces,
            package_type_names,
            exports,
        )?;
        exports.public_instances.push(PublicInstanceExport {
            name: public_path,
            module_path: receiver_unit.module_path.clone(),
            declared_receiver_type: receiver_type_ref(&receiver.symbol),
            implemented_interfaces: implemented_interfaces
                .iter()
                .map(|interface| interface.ty.clone())
                .collect(),
            operations: instance_operations,
        });
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PackagePublicInstanceReceiver<'a> {
    symbol: ServiceSymbolRef,
    unit: &'a FileIrUnit,
    decl: &'a skiff_artifact_model::TypeDeclIr,
}

#[derive(Debug, Clone)]
struct PackagePublicInstanceInterface {
    ty: TypeRefIr,
    instantiation: InterfaceInstantiationRef,
    methods: Vec<InterfaceMethodSignature>,
}

fn receiver_type_ref(receiver: &ServiceSymbolRef) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: receiver.clone(),
    }
}

fn public_instance_error(
    package: &PackageExportLinkProjectionInput<'_>,
    public_instance: &str,
    message: impl Into<String>,
) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: format!(
            "package {} public instance {}: {}",
            package.package_id,
            public_instance,
            message.into()
        ),
    }
}
