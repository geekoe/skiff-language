use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use skiff_artifact_model::{ContractLiteral, ContractTypeDescriptor, ContractTypeRef};
use skiff_runtime_linked_program::{
    LinkedNamedUnionBranch, LinkedNominalTypeRefBase, LinkedTypeDescriptor, LinkedTypeRef,
    RuntimeTypeContext, ServiceErrorDeclarationKind, ServiceErrorExecutionContext,
    ServiceErrorPublicIdentity, ServiceErrorTypeIndex, ServiceErrorTypeLink,
    SharedPackageLinkedImage, TypeAddr,
};

pub(super) fn build_service_error_type_index(
    shared: &SharedPackageLinkedImage,
    types: &RuntimeTypeContext,
) -> anyhow::Result<ServiceErrorTypeIndex> {
    let mut roots = Vec::new();
    let mut public_identity_by_addr = HashMap::new();

    for (code_slot, code) in shared.code_slots().iter().enumerate() {
        for (stable_key, entry) in &code.schema_index().types {
            let public_path = entry.public_path.as_deref().with_context(|| {
                format!(
                    "package {} schema entry {stable_key} has no exact public path",
                    code.artifact().package_id
                )
            })?;
            let export = code
                .artifact()
                .implementation_links
                .types
                .get(public_path)
                .with_context(|| {
                    format!(
                        "package {} public schema path {public_path} has no exact implementation type link",
                        code.artifact().package_id
                    )
                })?;
            let addr = types
                .exported_package_type(code_slot, public_path)
                .cloned()
                .with_context(|| {
                    format!(
                        "package {} public schema path {public_path} has no linked execution type",
                        code.artifact().package_id
                    )
                })?;
            let record = code
                .schema_records()
                .get(&entry.package_schema_type_id)
                .cloned()
                .with_context(|| {
                    format!(
                        "package {} schema entry {stable_key} is missing record {}",
                        code.artifact().package_id,
                        entry.package_schema_type_id
                    )
                })?;
            if record.package_id != code.artifact().package_id
                || record.stable_schema_key != *stable_key
                || record.package_schema_type_id != entry.package_schema_type_id
            {
                anyhow::bail!(
                    "package {} schema entry {stable_key} disagrees with its exact record",
                    code.artifact().package_id
                );
            }
            let public_identity = ServiceErrorPublicIdentity::new(
                record.package_id.clone(),
                record.stable_schema_key.clone(),
                record.package_schema_type_id.clone(),
            );
            if let Some(first) =
                public_identity_by_addr.insert(addr.clone(), public_identity.clone())
            {
                if first != public_identity {
                    anyhow::bail!(
                        "linked execution type {addr:?} maps to multiple public Package schema identities"
                    );
                }
            }
            roots.push(Root {
                code_slot,
                public_path: public_path.to_string(),
                export,
                addr,
                public_identity,
                record,
            });
        }
    }

    let mut links = Vec::new();
    for root in roots {
        validate_root(shared, types, &public_identity_by_addr, &root, &mut links)?;
    }
    ServiceErrorTypeIndex::try_new(links).map_err(anyhow::Error::new)
}

struct Root<'a> {
    code_slot: usize,
    public_path: String,
    export: &'a skiff_artifact_model::TypeExport,
    addr: TypeAddr,
    public_identity: ServiceErrorPublicIdentity,
    record: Arc<skiff_artifact_model::PackageSchemaTypeRecord>,
}

fn validate_root(
    shared: &SharedPackageLinkedImage,
    types: &RuntimeTypeContext,
    public_identity_by_addr: &HashMap<TypeAddr, ServiceErrorPublicIdentity>,
    root: &Root<'_>,
    links: &mut Vec<ServiceErrorTypeLink>,
) -> anyhow::Result<()> {
    let code = &shared.code_slots()[root.code_slot];
    let source_file = code
        .file(&root.export.file.file_ir_identity)
        .with_context(|| {
            format!(
                "package {} public schema path {} targets an unloaded source file",
                code.artifact().package_id,
                root.public_path
            )
        })?;
    let source_declaration = source_file
        .type_table
        .get(root.export.type_index as usize)
        .with_context(|| {
            format!(
                "package {} public schema path {} targets a missing source declaration",
                code.artifact().package_id,
                root.public_path
            )
        })?;
    let export_descriptor = root.export.descriptor.as_ref().with_context(|| {
        format!(
            "package {} public schema path {} has no exact exported descriptor",
            code.artifact().package_id,
            root.public_path
        )
    })?;
    if export_descriptor != &source_declaration.descriptor
        || root.export.type_params != source_declaration.type_params
    {
        anyhow::bail!(
            "package {} public schema path {} export descriptor disagrees with its execution declaration",
            code.artifact().package_id,
            root.public_path
        );
    }
    let linked_declaration = types.declaration(&root.addr).with_context(|| {
        format!(
            "package {} public schema path {} has no linked declaration",
            code.artifact().package_id,
            root.public_path
        )
    })?;
    if linked_declaration.name != source_declaration.name
        || linked_declaration.type_params != source_declaration.type_params
    {
        anyhow::bail!(
            "package {} public schema path {} linked declaration identity disagrees with File IR",
            code.artifact().package_id,
            root.public_path
        );
    }
    if !linked_declaration.type_params.is_empty()
        || !root.record.canonical_descriptor.type_params.is_empty()
    {
        anyhow::bail!(
            "generic public Package schema type {} is not admitted to ServiceErrorTypeIndex",
            root.public_path
        );
    }
    reject_applied_or_unresolved_types(&linked_declaration.descriptor)?;
    validate_descriptor_matches_schema(
        &linked_declaration.descriptor,
        &root.record.canonical_descriptor.descriptor,
        public_identity_by_addr,
    )
    .with_context(|| {
        format!(
            "package {} public schema path {} descriptor disagrees with its Package schema record",
            code.artifact().package_id,
            root.public_path
        )
    })?;

    match &linked_declaration.descriptor {
        LinkedTypeDescriptor::Record { .. } => {
            links.push(ServiceErrorTypeLink::try_new(
                root.public_identity.clone(),
                Arc::clone(&root.record),
                ServiceErrorExecutionContext::Declaration {
                    addr: root.addr.clone(),
                    kind: ServiceErrorDeclarationKind::Record,
                },
            )?);
        }
        LinkedTypeDescriptor::Representation { .. } => {
            links.push(ServiceErrorTypeLink::try_new(
                root.public_identity.clone(),
                Arc::clone(&root.record),
                ServiceErrorExecutionContext::Declaration {
                    addr: root.addr.clone(),
                    kind: ServiceErrorDeclarationKind::Representation,
                },
            )?);
        }
        LinkedTypeDescriptor::Union { branches } => {
            if branches.is_empty() {
                anyhow::bail!(
                    "public named union {} has no exact branch identity",
                    root.public_path
                );
            }
            for (branch_index, branch) in branches.iter().enumerate() {
                links.push(ServiceErrorTypeLink::try_new(
                    root.public_identity.clone(),
                    Arc::clone(&root.record),
                    ServiceErrorExecutionContext::NamedUnionBranch {
                        union_addr: root.addr.clone(),
                        branch_index,
                        branch: branch.clone(),
                        representation_owner: representation_owner(types, branch),
                    },
                )?);
            }
        }
        // Transparent aliases and interfaces are schema/link facts but are not
        // catch leaves, so they do not own a service-error execution row.
        LinkedTypeDescriptor::Alias { .. } | LinkedTypeDescriptor::Interface => {}
    }
    Ok(())
}

fn representation_owner(
    types: &RuntimeTypeContext,
    branch: &LinkedNamedUnionBranch,
) -> Option<TypeAddr> {
    let LinkedNamedUnionBranch::ConcreteNominal { nominal_type } = branch else {
        return None;
    };
    let addr = match nominal_type {
        LinkedTypeRef::Address { addr } => addr,
        LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr },
            ..
        } => addr,
        _ => return None,
    };
    matches!(
        types.descriptor(addr),
        Some(LinkedTypeDescriptor::Representation { .. })
    )
    .then(|| addr.clone())
}

fn reject_applied_or_unresolved_types(descriptor: &LinkedTypeDescriptor) -> anyhow::Result<()> {
    for ty in descriptor.type_refs() {
        reject_applied_or_unresolved_type(ty)?;
    }
    Ok(())
}

fn reject_applied_or_unresolved_type(ty: &LinkedTypeRef) -> anyhow::Result<()> {
    match ty {
        LinkedTypeRef::AppliedNominal { base, arguments } => {
            if matches!(base, LinkedNominalTypeRefBase::PackageSchema { .. }) {
                anyhow::bail!("applied PackageSchema is not admitted to ServiceErrorTypeIndex");
            }
            anyhow::bail!(
                "generic applied nominal is not admitted to ServiceErrorTypeIndex ({} arguments)",
                arguments.len()
            );
        }
        LinkedTypeRef::TypeParam { name } => {
            anyhow::bail!("unbound type parameter {name} is not admitted to ServiceErrorTypeIndex")
        }
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. } => {
            anyhow::bail!("unresolved nominal type is not admitted to ServiceErrorTypeIndex")
        }
        LinkedTypeRef::Native { args, .. } => {
            for argument in args {
                reject_applied_or_unresolved_type(argument)?;
            }
        }
        LinkedTypeRef::Record { fields } => {
            for field in fields.values() {
                reject_applied_or_unresolved_type(field)?;
            }
        }
        LinkedTypeRef::Union { items } => {
            for item in items {
                reject_applied_or_unresolved_type(item)?;
            }
        }
        LinkedTypeRef::Nullable { inner } => reject_applied_or_unresolved_type(inner)?,
        LinkedTypeRef::AnyInterface { interface } => {
            for argument in &interface.canonical_type_args {
                reject_applied_or_unresolved_type(argument)?;
            }
        }
        LinkedTypeRef::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                reject_applied_or_unresolved_type(&parameter.ty)?;
            }
            reject_applied_or_unresolved_type(return_type)?;
        }
        LinkedTypeRef::PackageSchema { .. }
        | LinkedTypeRef::DbObjectSymbol { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::Address { .. } => {}
    }
    Ok(())
}

fn validate_descriptor_matches_schema(
    linked: &LinkedTypeDescriptor,
    schema: &ContractTypeDescriptor,
    public_identity_by_addr: &HashMap<TypeAddr, ServiceErrorPublicIdentity>,
) -> anyhow::Result<()> {
    match (linked, schema) {
        (
            LinkedTypeDescriptor::Record {
                fields: linked_fields,
            },
            ContractTypeDescriptor::Record {
                fields: schema_fields,
            },
        ) => validate_fields(linked_fields, schema_fields, public_identity_by_addr),
        (
            LinkedTypeDescriptor::Representation { representation },
            ContractTypeDescriptor::Representation { target },
        )
        | (
            LinkedTypeDescriptor::Alias {
                target: representation,
            },
            ContractTypeDescriptor::Alias { target },
        ) => validate_type_matches_schema(representation, target, public_identity_by_addr),
        (
            LinkedTypeDescriptor::Union { branches },
            ContractTypeDescriptor::StructuralUnion { variants },
        ) => {
            if branches.len() != variants.len() {
                anyhow::bail!("named union branch count differs");
            }
            let mut remaining = variants.iter().collect::<Vec<_>>();
            for branch in branches {
                let Some(position) = remaining.iter().position(|variant| {
                    branch_matches_structural_schema(branch, variant, public_identity_by_addr)
                }) else {
                    anyhow::bail!("named union branch identity differs");
                };
                remaining.remove(position);
            }
            Ok(())
        }
        (
            LinkedTypeDescriptor::Union { branches },
            ContractTypeDescriptor::DiscriminatedUnion {
                discriminator_field,
                branches: schema_branches,
            },
        ) => {
            if branches.len() != schema_branches.len() {
                anyhow::bail!("discriminated union branch count differs");
            }
            for branch in branches {
                let LinkedNamedUnionBranch::SyntheticDiscriminator {
                    payload_type,
                    discriminator_field: linked_field,
                    discriminator_value,
                } = branch
                else {
                    anyhow::bail!("discriminated Package schema requires exact synthetic branches");
                };
                if linked_field != discriminator_field {
                    anyhow::bail!("discriminated union branch identity differs");
                }
                let schema_branch = schema_branches
                    .iter()
                    .find(|candidate| candidate.tag == *discriminator_value)
                    .context("discriminated union branch tag differs")?;
                validate_type_matches_schema(
                    payload_type,
                    &schema_branch.branch_type,
                    public_identity_by_addr,
                )?;
            }
            Ok(())
        }
        (
            LinkedTypeDescriptor::Union { branches },
            ContractTypeDescriptor::Enumeration { variants },
        ) => {
            if branches.len() != variants.len() {
                anyhow::bail!("enumeration branch count differs");
            }
            let mut linked_values = branches
                .iter()
                .map(|branch| {
                    let LinkedNamedUnionBranch::Literal {
                        value: skiff_artifact_model::LiteralIr::String { value },
                    } = branch
                    else {
                        anyhow::bail!("enumeration schema requires exact string literal branches");
                    };
                    Ok(value.as_str())
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let mut schema_values = variants.iter().map(String::as_str).collect::<Vec<_>>();
            linked_values.sort_unstable();
            schema_values.sort_unstable();
            if linked_values != schema_values {
                anyhow::bail!("enumeration branch identity differs");
            }
            Ok(())
        }
        (LinkedTypeDescriptor::Interface, ContractTypeDescriptor::CallbackInterface { .. }) => {
            Ok(())
        }
        _ => anyhow::bail!("declaration and Package schema descriptor kinds differ"),
    }
}

fn branch_matches_structural_schema(
    branch: &LinkedNamedUnionBranch,
    schema: &ContractTypeRef,
    public_identity_by_addr: &HashMap<TypeAddr, ServiceErrorPublicIdentity>,
) -> bool {
    match branch {
        LinkedNamedUnionBranch::ConcreteNominal { nominal_type } => {
            validate_type_matches_schema(nominal_type, schema, public_identity_by_addr).is_ok()
        }
        LinkedNamedUnionBranch::Literal { value } => contract_literal_as_linked(schema)
            .is_ok_and(|schema| matches!(schema, LinkedTypeRef::Literal { value: schema_value } if &schema_value == value)),
        LinkedNamedUnionBranch::SyntheticDiscriminator { .. } => false,
    }
}

fn validate_fields(
    linked: &std::collections::BTreeMap<String, LinkedTypeRef>,
    schema: &std::collections::BTreeMap<String, ContractTypeRef>,
    public_identity_by_addr: &HashMap<TypeAddr, ServiceErrorPublicIdentity>,
) -> anyhow::Result<()> {
    if linked.len() != schema.len() || linked.keys().ne(schema.keys()) {
        anyhow::bail!("record field names differ");
    }
    for (name, linked_type) in linked {
        validate_type_matches_schema(
            linked_type,
            schema
                .get(name)
                .expect("matching field key sets were validated"),
            public_identity_by_addr,
        )?;
    }
    Ok(())
}

fn validate_type_matches_schema(
    linked: &LinkedTypeRef,
    schema: &ContractTypeRef,
    public_identity_by_addr: &HashMap<TypeAddr, ServiceErrorPublicIdentity>,
) -> anyhow::Result<()> {
    match (linked, schema) {
        (
            LinkedTypeRef::Native {
                name: linked_name,
                args,
            },
            ContractTypeRef::Builtin {
                name: schema_name,
                arguments,
            },
        ) if linked_name == schema_name && args.len() == arguments.len() => {
            for (linked, schema) in args.iter().zip(arguments) {
                validate_type_matches_schema(linked, schema, public_identity_by_addr)?;
            }
            Ok(())
        }
        (
            LinkedTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            },
            ContractTypeRef::PackageSchema {
                package_id: schema_package,
                stable_schema_key: schema_key,
                package_schema_type_id: schema_type_id,
            },
        ) if package_id == schema_package
            && stable_schema_key == schema_key
            && package_schema_type_id == schema_type_id =>
        {
            Ok(())
        }
        (
            LinkedTypeRef::Address { addr },
            ContractTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            },
        ) => {
            let identity = public_identity_by_addr
                .get(addr)
                .context("linked address has no exact public Package schema identity")?;
            if identity.package_id() != package_id
                || identity.stable_schema_key() != stable_schema_key
                || identity.package_schema_type_id() != package_schema_type_id
            {
                anyhow::bail!("linked address resolves to a different Package schema identity");
            }
            Ok(())
        }
        (
            LinkedTypeRef::Record {
                fields: linked_fields,
            },
            ContractTypeRef::Record {
                fields: schema_fields,
            },
        ) => validate_fields(linked_fields, schema_fields, public_identity_by_addr),
        (LinkedTypeRef::Union { items }, ContractTypeRef::StructuralUnion { variants })
            if items.len() == variants.len() =>
        {
            for (linked, schema) in items.iter().zip(variants) {
                validate_type_matches_schema(linked, schema, public_identity_by_addr)?;
            }
            Ok(())
        }
        (
            LinkedTypeRef::Nullable { inner: linked },
            ContractTypeRef::Nullable { inner: schema },
        ) => validate_type_matches_schema(linked, schema, public_identity_by_addr),
        (
            LinkedTypeRef::Literal {
                value: skiff_artifact_model::LiteralIr::String { value: linked },
            },
            ContractTypeRef::Literal {
                value: ContractLiteral::String { value: schema },
            },
        ) if linked == schema => Ok(()),
        _ => anyhow::bail!("linked type and Package schema type differ"),
    }
}

fn contract_literal_as_linked(schema: &ContractTypeRef) -> anyhow::Result<LinkedTypeRef> {
    match schema {
        ContractTypeRef::Literal {
            value: ContractLiteral::String { value },
        } => Ok(LinkedTypeRef::Literal {
            value: skiff_artifact_model::LiteralIr::String {
                value: value.clone(),
            },
        }),
        _ => anyhow::bail!("named union literal branch differs from Package schema"),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use skiff_artifact_model::{
        AssemblyIdentity, CanonicalPackageLinkPlan, ContractTypeNameability, FileIrRef, FileIrUnit,
        NamedUnionBranchIr, PackageArtifact, PackageArtifactRef, PackageBuildId, PackageCodeSlot,
        PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
        PackageRuntimeRequirements, PackageSchemaCanonicalDescriptor, PackageSchemaIndex,
        PackageSchemaIndexEntry, PackageSchemaIndexRef, PackageSchemaTypeId,
        PackageSchemaTypeRecord, PackageSchemaTypeRecordRef, RuntimeAssembly,
        TypeDeclIr as ArtifactTypeDecl, TypeDescriptorIr, TypeExport,
        RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    };
    use skiff_runtime_linked_program::{
        FileAddr, HydratedPackageCode, LinkedTypeDescriptor, PackageCodeSlotIndex,
        PackageSymbolKey, PublicationResourceTable, TypeDeclIr, UnitAddr,
    };

    use super::*;

    #[test]
    fn exact_package_schema_reference_matches_linked_address_only_by_full_owner_identity() {
        let addr = TypeAddr {
            unit: skiff_runtime_linked_program::UnitAddr::Package(0),
            file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
            type_index: 0,
        };
        let type_id = PackageSchemaTypeId::new("type:fault");
        let identities = HashMap::from([(
            addr.clone(),
            ServiceErrorPublicIdentity::new("example/a", "Fault", type_id.clone()),
        )]);
        validate_type_matches_schema(
            &LinkedTypeRef::Address { addr: addr.clone() },
            &ContractTypeRef::package_schema("example/a", "Fault", type_id.clone()),
            &identities,
        )
        .unwrap();
        assert!(validate_type_matches_schema(
            &LinkedTypeRef::Address { addr },
            &ContractTypeRef::package_schema("example/b", "Fault", type_id),
            &identities,
        )
        .is_err());
    }

    #[test]
    fn builtin_schema_matching_remains_exact_without_alias_tolerance() {
        let public_identities = HashMap::new();
        let schema = ContractTypeRef::builtin("bool");
        validate_type_matches_schema(
            &LinkedTypeRef::Native {
                name: "bool".to_string(),
                args: Vec::new(),
            },
            &schema,
            &public_identities,
        )
        .expect("canonical builtin names should match exactly");

        let error = validate_type_matches_schema(
            &LinkedTypeRef::Native {
                name: "boolean".to_string(),
                args: Vec::new(),
            },
            &schema,
            &public_identities,
        )
        .expect_err("linker must reject an artificially noncanonical builtin pair");
        assert!(error.to_string().contains("differ"));
    }

    #[test]
    fn full_owner_indexes_build_without_operation_error_roots() {
        let own = record_package("example/service", "api.ServiceFault");
        let dependency = record_package("example/dependency", "api.DependencyFault");
        let (shared, types) = image([own, dependency]);

        let index = build_service_error_type_index(&shared, &types).unwrap();
        assert_eq!(index.public_identity_len(), 2);
        assert!(index.public_identities().any(|identity| {
            identity.package_id() == "example/service"
                && identity.stable_schema_key() == "api.ServiceFault"
        }));
        assert!(index.public_identities().any(|identity| {
            identity.package_id() == "example/dependency"
                && identity.stable_schema_key() == "api.DependencyFault"
        }));
    }

    #[test]
    fn representation_and_named_union_branch_context_are_retained() {
        let representation = package(
            "example/representation",
            "api.CodeFault",
            TypeDescriptorIr::Representation {
                representation: skiff_artifact_model::TypeRefIr::builtin("string"),
            },
            LinkedTypeDescriptor::Representation {
                representation: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            },
            ContractTypeDescriptor::Representation {
                target: ContractTypeRef::builtin("string"),
            },
            Vec::new(),
        );
        let union = package(
            "example/union",
            "api.UnionFault",
            TypeDescriptorIr::Union {
                branches: vec![
                    NamedUnionBranchIr::Literal {
                        value: skiff_artifact_model::LiteralIr::String {
                            value: "left".to_string(),
                        },
                    },
                    NamedUnionBranchIr::Literal {
                        value: skiff_artifact_model::LiteralIr::String {
                            value: "right".to_string(),
                        },
                    },
                ],
            },
            LinkedTypeDescriptor::Union {
                branches: vec![
                    LinkedNamedUnionBranch::Literal {
                        value: skiff_artifact_model::LiteralIr::String {
                            value: "left".to_string(),
                        },
                    },
                    LinkedNamedUnionBranch::Literal {
                        value: skiff_artifact_model::LiteralIr::String {
                            value: "right".to_string(),
                        },
                    },
                ],
            },
            ContractTypeDescriptor::Enumeration {
                variants: vec!["left".to_string(), "right".to_string()],
            },
            Vec::new(),
        );
        let (shared, types) = image([representation, union]);

        let index = build_service_error_type_index(&shared, &types).unwrap();
        let representation_identity = index
            .public_identities()
            .find(|identity| identity.package_id() == "example/representation")
            .unwrap()
            .clone();
        assert!(matches!(
            index.by_public_identity(&representation_identity).unwrap()[0].context(),
            ServiceErrorExecutionContext::Declaration {
                kind: ServiceErrorDeclarationKind::Representation,
                ..
            }
        ));
        let union_identity = index
            .public_identities()
            .find(|identity| identity.package_id() == "example/union")
            .unwrap()
            .clone();
        let union_links = index.by_public_identity(&union_identity).unwrap();
        assert_eq!(union_links.len(), 2);
        assert!(matches!(
            union_links[1].context(),
            ServiceErrorExecutionContext::NamedUnionBranch {
                branch_index: 1,
                branch: LinkedNamedUnionBranch::Literal { .. },
                ..
            }
        ));

        let representation_addr = TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index: 7,
        };
        let mut branch_types = RuntimeTypeContext::default();
        branch_types.descriptors.insert(
            representation_addr.clone(),
            TypeDeclIr {
                name: "BranchRepresentation".to_string(),
                descriptor: LinkedTypeDescriptor::Representation {
                    representation: LinkedTypeRef::Native {
                        name: "string".to_string(),
                        args: Vec::new(),
                    },
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            },
        );
        assert_eq!(
            representation_owner(
                &branch_types,
                &LinkedNamedUnionBranch::ConcreteNominal {
                    nominal_type: LinkedTypeRef::Address {
                        addr: representation_addr.clone(),
                    },
                },
            ),
            Some(representation_addr)
        );
    }

    #[test]
    fn missing_public_link_descriptor_mismatch_and_generic_public_fail_closed() {
        let mut missing_link = record_package("example/missing", "api.Missing");
        Arc::make_mut(&mut missing_link.artifact)
            .implementation_links
            .types
            .clear();
        let (shared, types) = image([missing_link]);
        assert!(build_service_error_type_index(&shared, &types)
            .unwrap_err()
            .to_string()
            .contains("implementation type link"));

        let mut descriptor_mismatch = record_package("example/mismatch", "api.Mismatch");
        Arc::make_mut(&mut descriptor_mismatch.record)
            .canonical_descriptor
            .descriptor = ContractTypeDescriptor::Enumeration {
            variants: vec!["wrong".to_string()],
        };
        let (shared, types) = image([descriptor_mismatch]);
        assert!(build_service_error_type_index(&shared, &types).is_err());

        let generic = package(
            "example/generic",
            "api.GenericFault",
            TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            ContractTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            vec!["T".to_string()],
        );
        let (shared, types) = image([generic]);
        assert!(build_service_error_type_index(&shared, &types)
            .unwrap_err()
            .to_string()
            .contains("generic public"));

        let dependency_type_id = PackageSchemaTypeId::new("schema:dependency");
        assert!(
            reject_applied_or_unresolved_type(&LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::PackageSchema {
                    package_id: "example/dependency".to_string(),
                    stable_schema_key: "api.Box".to_string(),
                    package_schema_type_id: dependency_type_id,
                },
                arguments: vec![LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                }],
            })
            .unwrap_err()
            .to_string()
            .contains("applied PackageSchema")
        );
    }

    struct PackageFixture {
        artifact_ref: PackageArtifactRef,
        artifact: Arc<PackageArtifact>,
        index: Arc<PackageSchemaIndex>,
        record: Arc<PackageSchemaTypeRecord>,
        file: Arc<FileIrUnit>,
        linked_descriptor: LinkedTypeDescriptor,
        stable_key: String,
        type_params: Vec<String>,
    }

    fn record_package(package_id: &str, stable_key: &str) -> PackageFixture {
        package(
            package_id,
            stable_key,
            TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            ContractTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
            Vec::new(),
        )
    }

    fn package(
        package_id: &str,
        stable_key: &str,
        source_descriptor: TypeDescriptorIr,
        linked_descriptor: LinkedTypeDescriptor,
        schema_descriptor: ContractTypeDescriptor,
        type_params: Vec<String>,
    ) -> PackageFixture {
        let fixture_type_params = type_params.clone();
        let canonical_descriptor = PackageSchemaCanonicalDescriptor {
            type_params: type_params.clone(),
            descriptor: schema_descriptor,
        };
        let type_id = skiff_artifact_identity::package_schema_type_id(
            package_id,
            stable_key,
            &canonical_descriptor,
        )
        .unwrap();
        let record = Arc::new(PackageSchemaTypeRecord {
            package_id: package_id.to_string(),
            stable_schema_key: stable_key.to_string(),
            package_schema_type_id: type_id.clone(),
            canonical_descriptor,
        });
        let index_types = BTreeMap::from([(
            stable_key.to_string(),
            PackageSchemaIndexEntry {
                package_schema_type_id: type_id.clone(),
                public_path: Some(stable_key.to_string()),
                nameability: ContractTypeNameability::PublicNameable,
            },
        )]);
        let index =
            Arc::new(PackageSchemaIndex {
                package_id: package_id.to_string(),
                package_schema_index_identity:
                    skiff_artifact_identity::package_schema_index_identity(package_id, &index_types)
                        .unwrap(),
                types: index_types,
            });
        let mut file = FileIrUnit::empty("errors", "source-hash");
        file.type_table.push(ArtifactTypeDecl {
            name: stable_key.to_string(),
            descriptor: source_descriptor.clone(),
            type_params: type_params.clone(),
            implements: Vec::new(),
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
        let file_ref = FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        };
        let artifact = Arc::new(PackageArtifact {
            schema_version: skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new(format!("build:{package_id}")),
            files: vec![file_ref.clone()],
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: package_id.to_string(),
                package_schema_index_identity: index.package_schema_index_identity.clone(),
            },
            package_schema_type_records: BTreeMap::from([(
                type_id.clone(),
                PackageSchemaTypeRecordRef {
                    package_id: package_id.to_string(),
                    package_schema_type_id: type_id,
                },
            )]),
            implementation_links: PackageImplementationLinks {
                types: BTreeMap::from([(
                    stable_key.to_string(),
                    TypeExport {
                        file: file_ref,
                        type_index: 0,
                        symbol: stable_key.to_string(),
                        is_interface: false,
                        descriptor: Some(source_descriptor),
                        type_params,
                        interface_methods: Vec::new(),
                    },
                )]),
                ..PackageImplementationLinks::default()
            },
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_roots: Vec::new(),
            service_call_refs: Vec::new(),
        });
        let artifact_ref = PackageArtifactRef {
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: artifact.package_build_id.clone(),
            package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
        };
        PackageFixture {
            artifact_ref,
            artifact,
            index,
            record,
            file: Arc::new(file),
            linked_descriptor,
            stable_key: stable_key.to_string(),
            type_params: fixture_type_params,
        }
    }

    fn image<const N: usize>(
        fixtures: [PackageFixture; N],
    ) -> (SharedPackageLinkedImage, RuntimeTypeContext) {
        let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("assembly:error-index"),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: fixtures
                .iter()
                .map(|fixture| fixture.artifact_ref.clone())
                .collect(),
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: fixtures
                    .iter()
                    .map(|fixture| PackageCodeSlot {
                        package: fixture.artifact_ref.clone(),
                    })
                    .collect(),
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let hydrated = fixtures
            .iter()
            .map(|fixture| {
                HydratedPackageCode::new(
                    Arc::clone(&fixture.artifact),
                    vec![Arc::clone(&fixture.file)],
                    PublicationResourceTable::default(),
                )
                .with_schema_index(Arc::clone(&fixture.index))
                .with_schema_records(BTreeMap::from([(
                    fixture.record.package_schema_type_id.clone(),
                    Arc::clone(&fixture.record),
                )]))
            })
            .collect::<Vec<_>>();
        let shared = SharedPackageLinkedImage::from_runtime_assembly(&assembly, hydrated).unwrap();
        let mut types = RuntimeTypeContext::default();
        for (code_slot, fixture) in fixtures.into_iter().enumerate() {
            let addr = TypeAddr {
                unit: UnitAddr::Package(code_slot),
                file: FileAddr::LoadedFileIndex(0),
                type_index: 0,
            };
            types.descriptors.insert(
                addr.clone(),
                TypeDeclIr {
                    name: fixture.stable_key.clone(),
                    descriptor: fixture.linked_descriptor,
                    type_params: fixture.type_params,
                    implements: Vec::new(),
                    source_span: None,
                },
            );
            types
                .exported_types
                .insert_package(PackageSymbolKey::new(code_slot, fixture.stable_key), addr);
            assert_eq!(
                shared
                    .code_by_slot(PackageCodeSlotIndex::new(code_slot))
                    .unwrap()
                    .code_slot(),
                PackageCodeSlotIndex::new(code_slot)
            );
        }
        (shared, types)
    }
}
