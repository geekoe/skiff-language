use std::collections::BTreeMap;

use skiff_artifact_model::{
    OperationTargetRef, PackageCallableId, PackageCallableSignature, PackageExportIndex,
    PackageImplementationLinks, PackageLocalAbiSymbol, PackageTypeRef,
};
use skiff_compiler_projection_input::ProjectionPackageCallableSignatureFacts;

use crate::{
    error::ProjectionError,
    package_artifact::{api_exports::PackageExports, export_links::ProjectedPackageExportLinks},
};

use super::signatures;

pub(super) struct LocalCallableSurface {
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_links: PackageImplementationLinks,
    pub callables: Vec<CanonicalCallable>,
}

pub(super) struct CanonicalCallable {
    pub public_path: String,
    pub callable_id: PackageCallableId,
    pub owner_module: String,
    pub executable_index: u32,
    pub signature: PackageCallableSignature,
    pub target: OperationTargetRef,
}

pub(super) struct CallableTarget {
    pub public_path: String,
    pub callable_id: PackageCallableId,
    pub owner_module: String,
    pub executable_index: u32,
    pub target: OperationTargetRef,
}

pub(super) fn project_local_surface(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &ProjectedPackageExportLinks,
    signatures: &ProjectionPackageCallableSignatureFacts,
) -> Result<LocalCallableSurface, ProjectionError> {
    let mut implementation_links = PackageImplementationLinks::from_exports(&exports.exports);
    for instance in &exports.public_instances {
        if implementation_links
            .constants
            .insert(instance.public_path.clone(), instance.receiver.clone())
            .is_some()
        {
            return Err(ProjectionError::InvalidPackageArtifact {
                message: format!(
                    "public instance {} receiver link conflicts with an exported constant",
                    instance.public_path
                ),
            });
        }
    }
    let mut public_symbols = project_non_callable_symbols(exports)?;
    let mut callable_targets = signatures::package_callable_targets(package_id, exports)?;
    signatures::add_direct_impl_method_targets(
        package_id,
        api_exports,
        &exports.exports,
        &mut callable_targets,
    )?;
    let callables =
        signatures::attach_canonical_signatures(package_id, signatures, callable_targets)?;
    add_public_instance_symbols(exports, &callables, &mut public_symbols)?;
    Ok(LocalCallableSurface {
        public_symbols,
        implementation_links,
        callables,
    })
}

fn project_non_callable_symbols(
    projected: &ProjectedPackageExportLinks,
) -> Result<BTreeMap<String, PackageLocalAbiSymbol>, ProjectionError> {
    let mut symbols = BTreeMap::new();
    let exports: &PackageExportIndex = &projected.exports;
    for (public_path, export) in &exports.types {
        let descriptor =
            export
                .descriptor
                .clone()
                .ok_or_else(|| ProjectionError::InvalidPackageArtifact {
                    message: format!("package type export {public_path} has no typed descriptor"),
                })?;
        insert_public_symbol(
            &mut symbols,
            public_path.clone(),
            PackageLocalAbiSymbol::Type {
                local_type_id: format!("type:{public_path}"),
                descriptor,
                is_alias: projected.alias_types.contains(public_path),
                is_interface: export.is_interface,
                type_params: export.type_params.clone(),
                interface_methods: export.interface_methods.clone(),
                actor: export.actor.clone(),
            },
        )?;
    }
    for (public_path, export) in &exports.constants {
        insert_public_symbol(
            &mut symbols,
            public_path.clone(),
            PackageLocalAbiSymbol::Constant {
                const_id: format!("const:{public_path}"),
                ty: PackageTypeRef::Local {
                    local_type: export.ty.clone(),
                },
            },
        )?;
    }
    Ok(symbols)
}

fn add_public_instance_symbols(
    exports: &ProjectedPackageExportLinks,
    callables: &[CanonicalCallable],
    public_symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
) -> Result<(), ProjectionError> {
    let callable_ids = callables
        .iter()
        .map(|callable| (callable.public_path.as_str(), callable.callable_id.clone()))
        .collect::<BTreeMap<_, _>>();
    for instance in &exports.public_instances {
        let methods = instance
            .methods
            .iter()
            .map(|method| {
                let callable_id = callable_ids
                    .get(method.public_path.as_str())
                    .cloned()
                    .ok_or_else(|| ProjectionError::InvalidPackageArtifact {
                        message: format!(
                            "public instance {} method {} has no Local ABI callable",
                            instance.public_path, method.public_path
                        ),
                    })?;
                Ok((method.name.clone(), callable_id))
            })
            .collect::<Result<BTreeMap<_, _>, ProjectionError>>()?;
        insert_public_symbol(
            public_symbols,
            instance.public_path.clone(),
            PackageLocalAbiSymbol::PublicInstance {
                instance_id: instance.public_path.clone(),
                declared_receiver_type: instance.declared_receiver_type.clone(),
                interfaces: instance.interfaces.clone(),
                methods,
            },
        )?;
    }
    Ok(())
}

pub(super) fn insert_public_symbol(
    symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
    public_path: String,
    symbol: PackageLocalAbiSymbol,
) -> Result<(), ProjectionError> {
    if symbols.insert(public_path.clone(), symbol).is_some() {
        return Err(ProjectionError::InvalidPackageArtifact {
            message: format!("duplicate PackageLocalAbi public path {public_path}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
