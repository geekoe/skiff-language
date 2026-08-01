use skiff_artifact_model::{
    ActorAbiInput, ActorCreateSignatureIr, ActorDeclarationIr, ActorFieldIr, ActorPublicMethodIr,
    FunctionTypeParamIr, PackageActorAbi, TypeRefIr,
};

use crate::error::ProjectionError;

/// Projects one lowered actor declaration into the package ABI shape, applying
/// the caller's artifact-view normalization to every type reference. The ABI
/// identity is carried verbatim from the declaration; the normalized `abi`
/// surface is what dependents resolve through `actor_type_resolution`.
pub(super) fn project_actor_abi(
    declaration: &ActorDeclarationIr,
    normalize_type: impl Fn(&TypeRefIr) -> Result<TypeRefIr, ProjectionError>,
) -> Result<PackageActorAbi, ProjectionError> {
    let normalize_parameters = |parameters: &[FunctionTypeParamIr]| {
        parameters
            .iter()
            .map(|parameter| {
                Ok(FunctionTypeParamIr {
                    name: parameter.name.clone(),
                    ty: normalize_type(&parameter.ty)?,
                })
            })
            .collect::<Result<Vec<_>, ProjectionError>>()
    };
    Ok(PackageActorAbi {
        actor_abi_identity: declaration.actor_abi_identity.clone(),
        abi: ActorAbiInput {
            actor_name: declaration.abi.actor_name.clone(),
            actor_id_type: normalize_type(&declaration.abi.actor_id_type)?,
            key_field: declaration.abi.key_field.clone(),
            fields: declaration
                .abi
                .fields
                .iter()
                .map(|field| {
                    Ok(ActorFieldIr {
                        name: field.name.clone(),
                        ty: normalize_type(&field.ty)?,
                        encoding: field.encoding,
                    })
                })
                .collect::<Result<Vec<_>, ProjectionError>>()?,
            create: declaration
                .abi
                .create
                .as_ref()
                .map(|create| {
                    Ok(ActorCreateSignatureIr {
                        parameters: normalize_parameters(&create.parameters)?,
                    })
                })
                .transpose()?,
            public_methods: declaration
                .abi
                .public_methods
                .iter()
                .map(|method| {
                    Ok(ActorPublicMethodIr {
                        method_identity: method.method_identity.clone(),
                        name: method.name.clone(),
                        parameters: normalize_parameters(&method.parameters)?,
                        return_type: normalize_type(&method.return_type)?,
                        may_suspend: method.may_suspend,
                    })
                })
                .collect::<Result<Vec<_>, ProjectionError>>()?,
            actor_runtime_abi_version: declaration.abi.actor_runtime_abi_version.clone(),
        },
    })
}
