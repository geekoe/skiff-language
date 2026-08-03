use super::*;

pub(crate) fn unknown_plan_for_type_ref(type_ref: &LinkedTypeRef) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: linked_type_ref_label(type_ref).to_string(),
        named_type_name: linked_type_ref_named_type_name(type_ref),
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Unknown,
    }
}

pub(crate) fn unknown_plan_for_descriptor(descriptor: &LinkedTypeDescriptor) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: linked_type_descriptor_label(descriptor).to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Unknown,
    }
}

pub(crate) fn linked_type_ref_kind(type_ref: &LinkedTypeRef) -> &'static str {
    match type_ref {
        LinkedTypeRef::LocalType { .. } => "localType",
        LinkedTypeRef::PublicationType { .. } => "publicationType",
        LinkedTypeRef::ServiceSymbol { .. } => "serviceSymbol",
        LinkedTypeRef::PackageSymbol { .. } => "packageSymbol",
        LinkedTypeRef::PackageSchema { .. } => "packageSchema",
        LinkedTypeRef::AppliedNominal { .. } => "appliedNominal",
        LinkedTypeRef::Address { .. } => "address",
        LinkedTypeRef::Native { .. } => "builtin",
        LinkedTypeRef::Record { .. } => "record",
        LinkedTypeRef::Union { .. } => "union",
        LinkedTypeRef::Nullable { .. } => "nullable",
        LinkedTypeRef::Literal { .. } => "literal",
        LinkedTypeRef::TypeParam { .. } => "typeParam",
        LinkedTypeRef::Function { .. } => "function",
        LinkedTypeRef::DbObjectSymbol { .. } => "dbObjectSymbol",
        LinkedTypeRef::AnyInterface { .. } => "anyInterface",
    }
}

pub(crate) fn linked_type_ref_label(type_ref: &LinkedTypeRef) -> &'static str {
    match type_ref {
        LinkedTypeRef::Native { .. } => "builtin",
        LinkedTypeRef::LocalType { .. } => "localType",
        LinkedTypeRef::PublicationType { .. } => "publicationType",
        LinkedTypeRef::ServiceSymbol { .. } => "serviceSymbol",
        LinkedTypeRef::PackageSymbol { .. } => "packageSymbol",
        LinkedTypeRef::PackageSchema { .. } => "packageSchema",
        LinkedTypeRef::AppliedNominal { .. } => "appliedNominal",
        LinkedTypeRef::Address { .. } => "address",
        LinkedTypeRef::Record { .. } => "record",
        LinkedTypeRef::Union { .. } => "union",
        LinkedTypeRef::Nullable { .. } => "nullable",
        LinkedTypeRef::Literal { .. } => "literal",
        LinkedTypeRef::TypeParam { .. } => "typeParam",
        LinkedTypeRef::Function { .. } => "function",
        LinkedTypeRef::DbObjectSymbol { .. } => "dbObjectSymbol",
        LinkedTypeRef::AnyInterface { .. } => "anyInterface",
    }
}

pub(crate) fn linked_type_ref_named_type_name(type_ref: &LinkedTypeRef) -> Option<String> {
    match type_ref {
        LinkedTypeRef::Native { name, .. } => Some(name.clone()),
        _ => None,
    }
}

pub(crate) fn linked_type_descriptor_label(descriptor: &LinkedTypeDescriptor) -> &'static str {
    match descriptor {
        LinkedTypeDescriptor::Record { .. } => "record",
        LinkedTypeDescriptor::Representation { .. } => "representation",
        LinkedTypeDescriptor::Alias { .. } => "alias",
        LinkedTypeDescriptor::Union { .. } => "union",
        LinkedTypeDescriptor::Interface => "interface",
    }
}

pub(crate) fn artifact_type_ref_label(type_ref: &skiff_artifact_model::TypeRefIr) -> &'static str {
    use skiff_artifact_model::TypeRefIr;
    match type_ref {
        TypeRefIr::Builtin { .. } => "builtin",
        TypeRefIr::LocalType { .. } => "localType",
        TypeRefIr::PublicationType { .. } => "publicationType",
        TypeRefIr::ServiceSymbol { .. } => "serviceSymbol",
        TypeRefIr::PackageSymbol { .. } => "packageSymbol",
        TypeRefIr::PackageSchema { .. } => "packageSchema",
        TypeRefIr::AppliedNominal { .. } => "appliedNominal",
        TypeRefIr::DbObjectSymbol { .. } => "dbObjectSymbol",
        TypeRefIr::Record { .. } => "record",
        TypeRefIr::Union { .. } => "union",
        TypeRefIr::Nullable { .. } => "nullable",
        TypeRefIr::Literal { .. } => "literal",
        TypeRefIr::TypeParam { .. } => "typeParam",
        TypeRefIr::Function { .. } => "function",
        TypeRefIr::AnyInterface { .. } => "anyInterface",
    }
}

pub(crate) fn artifact_type_ref_named_type_name(
    type_ref: &skiff_artifact_model::TypeRefIr,
) -> Option<String> {
    match type_ref {
        skiff_artifact_model::TypeRefIr::Builtin { name, .. } => Some(name.clone()),
        _ => None,
    }
}
