mod interfaces;
mod operations;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    ConstExport, ExecutableExport, FileIrRef, FileIrUnit, PackageExportIndex, ServiceSymbolRef,
    TypeRefIr,
};

use crate::{
    error::ProjectionError,
    package_artifact::visible_types::{projection_visible_type_ref, PackageVisibleTypeNames},
};

use super::package_scoped_export_symbol;
use crate::package_artifact::model::PackageExportLinkProjectionInput;

pub(super) fn project_package_public_instances(
    package: &PackageExportLinkProjectionInput<'_>,
    files_by_module: &BTreeMap<String, FileIrRef>,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    package_type_names: &PackageVisibleTypeNames,
    exports: &mut PackageExportIndex,
) -> Result<Vec<PackagePublicInstanceExecutionLink>, ProjectionError> {
    let mut seen_instances = BTreeMap::<String, String>::new();
    let mut projected = Vec::new();
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
        let receiver_const = ConstExport {
            file: receiver_file,
            const_index: const_decl.const_index,
            symbol: public_instance.const_symbol.clone(),
            ty: projection_visible_type_ref(
                &public_instance.module,
                &constant.ty,
                package_type_names,
            ),
        };
        let receiver = interfaces::resolve_receiver(
            package,
            file_units_by_module,
            &public_path,
            &public_instance.module,
            &constant.ty,
            &receiver_const.ty,
            &public_instance.receiver_module,
            &public_instance.receiver_symbol,
        )?;
        let implemented_interfaces = interfaces::resolve_interfaces(
            package,
            file_units_by_module,
            &public_path,
            &public_instance.interfaces,
        )?;
        let instance_operations = operations::project_operations(
            package,
            files_by_module,
            file_units_by_module,
            &public_path,
            &receiver,
            &implemented_interfaces,
            package_type_names,
            exports,
        )?;
        projected.push(PackagePublicInstanceExecutionLink {
            public_path,
            declared_receiver_type: receiver_type_ref(&receiver.symbol),
            interfaces: implemented_interfaces
                .iter()
                .map(|interface| interface.ty.clone())
                .collect(),
            receiver: receiver_const,
            methods: instance_operations,
        });
    }
    Ok(projected)
}

#[derive(Debug, Clone)]
pub(in crate::package_artifact) struct PackagePublicInstanceExecutionLink {
    pub public_path: String,
    pub declared_receiver_type: TypeRefIr,
    pub interfaces: Vec<TypeRefIr>,
    pub receiver: ConstExport,
    pub methods: Vec<PackagePublicInstanceMethodExecutionLink>,
}

#[derive(Debug, Clone)]
pub(in crate::package_artifact) struct PackagePublicInstanceMethodExecutionLink {
    pub name: String,
    pub public_path: String,
    pub executable: ExecutableExport,
}

#[derive(Debug, Clone)]
struct PackagePublicInstanceReceiver {
    symbol: ServiceSymbolRef,
    type_params: Vec<String>,
}

#[derive(Debug, Clone)]
struct PackagePublicInstanceInterface {
    ty: TypeRefIr,
    methods: Vec<crate::package_artifact::api_exports::PackageExportPublicInstanceMethod>,
}

fn receiver_type_ref(receiver: &ServiceSymbolRef) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: receiver.clone(),
    }
}

impl PackagePublicInstanceReceiver {
    fn definition_type(&self) -> TypeRefIr {
        if self.type_params.is_empty() {
            return receiver_type_ref(&self.symbol);
        }
        TypeRefIr::AppliedNominal {
            base: skiff_artifact_model::NominalTypeRefBaseIr::ServiceSymbol {
                symbol: self.symbol.clone(),
            },
            arguments: self
                .type_params
                .iter()
                .map(|name| TypeRefIr::TypeParam { name: name.clone() })
                .collect(),
        }
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
