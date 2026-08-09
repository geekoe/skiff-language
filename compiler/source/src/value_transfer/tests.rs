use std::collections::BTreeMap;

use skiff_artifact_model::{
    NativeValueLifecycleRegistry, NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef,
    TypeDescriptorIr, TypeRefIr, ValueDropPlan, ValueTransferPlan,
};

use super::*;

mod generics;
mod native_registry;
mod plans;
mod rejection;

pub(super) fn builtin(name: &str) -> TypeRefIr {
    TypeRefIr::builtin(name)
}

pub(super) fn generic_builtin(name: &str, args: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args,
    }
}

pub(super) fn local(type_index: u32) -> TypeRefIr {
    TypeRefIr::LocalType { type_index }
}

pub(super) fn local_id(type_index: u32) -> SourceValueTransferNominalId {
    SourceValueTransferNominalId::Local {
        module_path: "app.model".to_string(),
        type_index,
    }
}

pub(super) fn ordinary_fact(
    type_parameters: &[&str],
    descriptor: TypeDescriptorIr,
) -> SourceValueTransferNominalFact {
    SourceValueTransferNominalFact {
        declaration_module: "app.model".to_string(),
        type_parameters: type_parameters
            .iter()
            .map(|parameter| (*parameter).to_string())
            .collect(),
        semantics: SourceValueTransferNominalSemantics::Ordinary(descriptor),
    }
}

pub(super) fn applied_local(type_index: u32, arguments: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index },
        arguments,
    }
}

pub(super) fn exact_package_type(
    symbol_path: &str,
    abi: &str,
    arguments: Vec<TypeRefIr>,
) -> TypeRefIr {
    let symbol = PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: "pkg.lifecycle".to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: Some(abi.to_string()),
    };
    if arguments.is_empty() {
        TypeRefIr::PackageSymbol { symbol }
    } else {
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol { symbol },
            arguments,
        }
    }
}

pub(super) fn plan(
    facts: &SourceValueTransferFacts,
    ty: &TypeRefIr,
) -> Result<ValueTransferPlan, SourceValueTransferError> {
    facts.plan(SourceValueTransferPlanInput::concrete("app.model", ty))
}

pub(super) fn relocatable_plan(
    facts: &SourceValueTransferFacts,
    ty: &TypeRefIr,
    binders: &[String],
) -> Result<ValueTransferPlan, SourceValueTransferError> {
    facts.plan(SourceValueTransferPlanInput::relocatable(
        "app.model",
        ty,
        binders,
    ))
}

pub(super) fn plan_with_registry(
    facts: &SourceValueTransferFacts,
    registry: &NativeValueLifecycleRegistry,
    ty: &TypeRefIr,
    binders: &[String],
) -> Result<ValueTransferPlan, SourceValueTransferError> {
    facts.plan_with_registry(
        registry,
        SourceValueTransferPlanInput::relocatable("app.model", ty, binders),
    )
}

pub(super) fn root_error(error: &SourceValueTransferError) -> &SourceValueTransferError {
    match error {
        SourceValueTransferError::AtStructuralPosition { source, .. } => root_error(source),
        other => other,
    }
}

pub(super) fn snapshot_release() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    }
}

pub(super) fn assert_no_recursive_shape(plan: &ValueTransferPlan) {
    match plan {
        ValueTransferPlan::SnapshotShare { drop } | ValueTransferPlan::MoveOnly { drop } => {
            assert!(!matches!(drop, ValueDropPlan::RecursiveShape { .. }));
        }
        ValueTransferPlan::AffineResource { drop }
        | ValueTransferPlan::ExplicitCloneLease { drop, .. } => {
            assert!(!matches!(
                drop,
                skiff_artifact_model::ResourceDropPlan::RecursiveShape { .. }
            ));
        }
        ValueTransferPlan::FromType { .. } => {}
    }
}

pub(super) fn record(fields: impl IntoIterator<Item = (&'static str, TypeRefIr)>) -> TypeRefIr {
    TypeRefIr::Record {
        fields: fields
            .into_iter()
            .map(|(name, ty)| (name.to_string(), ty))
            .collect::<BTreeMap<_, _>>(),
    }
}
