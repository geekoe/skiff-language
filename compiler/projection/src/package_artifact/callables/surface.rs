use std::collections::BTreeMap;

use skiff_artifact_model::{
    OperationTargetRef, PackageCallableId, PackageCallableSignature, PackageExportIndex,
    PackageImplementationLinks, PackageLocalAbiSymbol, PackageTypeRef,
};
use skiff_compiler_projection_input::ProjectionPackageCallableSignatureFacts;

use crate::{error::ProjectionError, package_artifact::api_exports::PackageExports};

use super::signatures;

pub(super) struct LocalCallableSurface {
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_links: PackageImplementationLinks,
    pub callables: Vec<CallableSeed>,
}

pub(super) struct CallableSeed {
    pub public_path: String,
    pub callable_id: PackageCallableId,
    pub owner_module: String,
    pub executable_index: u32,
    pub signature: PackageCallableSignature,
    pub target: OperationTargetRef,
}

pub(super) fn project_local_surface(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &PackageExportIndex,
    signatures: &ProjectionPackageCallableSignatureFacts,
) -> Result<LocalCallableSurface, ProjectionError> {
    let implementation_links = PackageImplementationLinks::from_exports(exports);
    let mut public_symbols = project_non_callable_symbols(exports)?;
    let mut callables = signatures::package_callable_seeds(package_id, exports);
    signatures::add_direct_impl_method_seeds(package_id, api_exports, exports, &mut callables);
    signatures::attach_canonical_signatures(package_id, signatures, &mut callables)?;
    add_public_instance_symbols(exports, &callables, &mut public_symbols)?;
    Ok(LocalCallableSurface {
        public_symbols,
        implementation_links,
        callables,
    })
}

fn project_non_callable_symbols(
    exports: &PackageExportIndex,
) -> Result<BTreeMap<String, PackageLocalAbiSymbol>, ProjectionError> {
    let mut symbols = BTreeMap::new();
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
    exports: &PackageExportIndex,
    seeds: &[CallableSeed],
    public_symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
) -> Result<(), ProjectionError> {
    let callable_ids = seeds
        .iter()
        .map(|seed| (seed.public_path.as_str(), seed.callable_id.clone()))
        .collect::<BTreeMap<_, _>>();
    for instance in &exports.public_instances {
        let methods = instance
            .operations
            .iter()
            .map(|operation| {
                let public_path = operation.operation.public_path.as_str();
                let method_name = public_path
                    .strip_prefix(instance.name.as_str())
                    .and_then(|suffix| suffix.strip_prefix('.'))
                    .unwrap_or(public_path)
                    .to_string();
                let callable_id = callable_ids.get(public_path).cloned().ok_or_else(|| {
                    ProjectionError::InvalidPackageArtifact {
                        message: format!(
                            "public instance {} method {public_path} has no Local ABI callable",
                            instance.name
                        ),
                    }
                })?;
                Ok((method_name, callable_id))
            })
            .collect::<Result<BTreeMap<_, _>, ProjectionError>>()?;
        insert_public_symbol(
            public_symbols,
            instance.name.clone(),
            PackageLocalAbiSymbol::PublicInstance {
                instance_id: instance.name.clone(),
                declared_receiver_type: instance.declared_receiver_type.clone(),
                interfaces: instance.implemented_interfaces.clone(),
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
