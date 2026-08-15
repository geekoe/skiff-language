use skiff_artifact_model::{PackageLocalAbiSymbol, PackageRefIr, TypeDescriptorIr, TypeRefIr};
use skiff_runtime_linked_bytecode::LinkedValueTransferPlan;
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::normalization::normalize_type;

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_representation_carrier(
    deployment: &HydratedDeploymentBytecode,
    caller: &HydratedBytecodePackage,
    owner_type: &TypeRefIr,
    representation_type: &TypeRefIr,
    physical_carrier_type: &TypeRefIr,
    owner_plan: &LinkedValueTransferPlan,
    representation_plan: &LinkedValueTransferPlan,
    physical_carrier_plan: &LinkedValueTransferPlan,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if owner_plan != representation_plan || owner_plan != physical_carrier_plan {
        return Err(obligation_error(
            BytecodeLinkObligation::FrameAndValueTransferPlan,
            location,
            "representation owner, source payload, and physical carrier plans differ".to_string(),
        ));
    }

    let TypeRefIr::PackageSymbol { symbol } = owner_type else {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation carrier owner is not an exact package symbol".to_string(),
        ));
    };
    let PackageRefIr::PackageId { package_id } = &symbol.package else {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation carrier owner retains a dependency alias".to_string(),
        ));
    };
    let mut owners = deployment
        .packages()
        .values()
        .filter(|package| package.reference().package_id == *package_id);
    let owner = owners.next().ok_or_else(|| {
        obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location.clone(),
            "representation carrier package owner is absent".to_string(),
        )
    })?;
    if owners.next().is_some() {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation carrier package owner is ambiguous".to_string(),
        ));
    }
    if symbol.abi_expectation.as_deref()
        != Some(owner.reference().package_local_abi_identity.as_str())
    {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation carrier owner ABI does not match the exact package".to_string(),
        ));
    }

    let abi = &owner.artifact().package_local_abi;
    let descriptor = if owner.reference().package_build_id == caller.reference().package_build_id {
        abi.implementation_symbols
            .get(&symbol.symbol_path)
            .or_else(|| abi.public_symbols.get(&symbol.symbol_path))
    } else {
        abi.public_symbols
            .get(&symbol.symbol_path)
            .or_else(|| abi.implementation_symbols.get(&symbol.symbol_path))
    }
    .ok_or_else(|| {
        obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location.clone(),
            "representation carrier owner is absent from its exact package ABI".to_string(),
        )
    })?;
    let PackageLocalAbiSymbol::Type {
        descriptor: TypeDescriptorIr::Representation { representation },
        is_alias: false,
        is_interface: false,
        type_params,
        interface_methods,
        actor: None,
        ..
    } = descriptor
    else {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation carrier owner ABI is not an exact non-alias Representation type"
                .to_string(),
        ));
    };
    if !type_params.is_empty() || !interface_methods.is_empty() {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation carrier owner ABI is generic or callable".to_string(),
        ));
    }

    let exact_representation = normalize_type(deployment, owner, representation, &location)?;
    if &exact_representation != representation_type {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation carrier source row differs from its exact ABI descriptor payload"
                .to_string(),
        ));
    }
    if !matches!(
        representation_type,
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty()
    ) {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation source payload is not exact builtin integer".to_string(),
        ));
    }
    if !matches!(
        physical_carrier_type,
        TypeRefIr::Builtin { name, args } if name == "number" && args.is_empty()
    ) {
        return Err(obligation_error(
            BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            location,
            "representation physical carrier is not exact builtin number".to_string(),
        ));
    }
    Ok(())
}

fn obligation_error(
    obligation: BytecodeLinkObligation,
    location: BytecodeLinkLocation,
    detail: String,
) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation,
        location,
        detail,
    }
}
