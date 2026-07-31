use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use skiff_artifact_model::{
    ContractLiteral, ContractTypeDescriptor, ContractTypeRef, FileIrRef, PackageRefIr,
    TypeDescriptorIr,
};
use skiff_runtime_linked_program::{
    type_descriptor_to_value, FileAddr, LinkedNamedUnionBranch, LinkedNominalTypeRefBase,
    LinkedTypeDescriptor, LinkedTypeRef, RuntimeTypeContext, ServiceErrorDeclarationKind,
    ServiceErrorExecutionContext, ServiceErrorPublicIdentity, ServiceErrorTypeIndex,
    ServiceErrorTypeLink, ServiceSymbolRef, SharedPackageLinkedImage, TypeAddr, UnitAddr,
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
    let coordinates = ExactTypeCoordinateResolver::new(shared, types);
    let source_file_index = coordinates
        .exact_file_index(root.code_slot, &root.export.file)
        .with_context(|| {
            format!(
                "package {} public schema path {} targets an unloaded or non-exact source file",
                code.artifact().package_id,
                root.public_path
            )
        })?;
    let exact_root_addr = coordinates
        .type_addr(
            root.code_slot,
            source_file_index,
            root.export.type_index as usize,
        )
        .with_context(|| {
            format!(
                "package {} public schema path {} targets a missing source declaration",
                code.artifact().package_id,
                root.public_path
            )
        })?;
    if exact_root_addr != root.addr {
        anyhow::bail!(
            "package {} public schema path {} linked execution coordinate disagrees with its exact export coordinate",
            code.artifact().package_id,
            root.public_path
        );
    }
    let source_file = &code.files()[source_file_index];
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
    let linked_declaration = types.declaration(&root.addr).with_context(|| {
        format!(
            "package {} public schema path {} has no linked declaration",
            code.artifact().package_id,
            root.public_path
        )
    })?;
    if root.export.type_params != source_declaration.type_params
        || source_declaration.type_params != linked_declaration.type_params
    {
        anyhow::bail!(
            "package {} public schema path {} export descriptor disagrees with its execution declaration",
            code.artifact().package_id,
            root.public_path
        );
    }
    if linked_declaration.name != source_declaration.name {
        anyhow::bail!(
            "package {} public schema path {} linked declaration identity disagrees with File IR",
            code.artifact().package_id,
            root.public_path
        );
    }
    validate_artifact_descriptor_matches_linked(
        &coordinates,
        root.code_slot,
        source_file_index,
        &source_declaration.descriptor,
        &linked_declaration.descriptor,
    )
    .with_context(|| {
        format!(
            "package {} public schema path {} execution declaration disagrees with its canonical linked coordinate",
            code.artifact().package_id,
            root.public_path
        )
    })?;
    validate_artifact_descriptor_matches_linked(
        &coordinates,
        root.code_slot,
        source_file_index,
        export_descriptor,
        &linked_declaration.descriptor,
    )
    .with_context(|| {
        format!(
            "package {} public schema path {} export descriptor disagrees with its execution declaration",
            code.artifact().package_id,
            root.public_path
        )
    })?;
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

struct ExactTypeCoordinateResolver<'a> {
    shared: &'a SharedPackageLinkedImage,
    types: &'a RuntimeTypeContext,
}

impl<'a> ExactTypeCoordinateResolver<'a> {
    fn new(shared: &'a SharedPackageLinkedImage, types: &'a RuntimeTypeContext) -> Self {
        Self { shared, types }
    }

    fn exact_file_index(&self, code_slot: usize, file_ref: &FileIrRef) -> anyhow::Result<usize> {
        let code = self
            .shared
            .code_slots()
            .get(code_slot)
            .with_context(|| format!("package code slot {code_slot} is out of bounds"))?;
        let mut matches = code
            .files()
            .iter()
            .enumerate()
            .filter(|(_, file)| file.file_ir_identity == file_ref.file_ir_identity);
        let (file_index, file) = matches
            .next()
            .context("export File IR identity is not loaded")?;
        if matches.next().is_some()
            || file.module_path != file_ref.module_path
            || file_ref
                .source_ast_hash
                .as_deref()
                .is_some_and(|hash| hash != file.source_ast_hash)
        {
            anyhow::bail!("export File IR ref does not exactly match loaded code");
        }
        Ok(file_index)
    }

    fn type_addr(
        &self,
        code_slot: usize,
        file_index: usize,
        type_index: usize,
    ) -> anyhow::Result<TypeAddr> {
        let code = self
            .shared
            .code_slots()
            .get(code_slot)
            .with_context(|| format!("package code slot {code_slot} is out of bounds"))?;
        let file = code
            .files()
            .get(file_index)
            .with_context(|| format!("file index {file_index} is out of bounds"))?;
        let source = file.type_table.get(type_index).with_context(|| {
            format!(
                "type index {type_index} is out of bounds for {}",
                file.file_ir_identity
            )
        })?;
        let addr = TypeAddr {
            unit: UnitAddr::Package(code_slot),
            file: FileAddr::LoadedFileIndex(file_index),
            type_index,
        };
        let linked = self
            .types
            .declaration(&addr)
            .context("exact type coordinate has no linked declaration")?;
        if linked.name != source.name || linked.type_params != source.type_params {
            anyhow::bail!("exact type coordinate declaration identity disagrees with File IR");
        }
        Ok(addr)
    }

    fn publication_type_addr(
        &self,
        code_slot: usize,
        module_path: &str,
        type_index: usize,
    ) -> anyhow::Result<TypeAddr> {
        let code = self
            .shared
            .code_slots()
            .get(code_slot)
            .with_context(|| format!("package code slot {code_slot} is out of bounds"))?;
        let mut matches = code
            .files()
            .iter()
            .enumerate()
            .filter(|(_, file)| file.module_path == module_path);
        let (file_index, _) = matches
            .next()
            .with_context(|| format!("module {module_path} is unresolved"))?;
        if matches.next().is_some() {
            anyhow::bail!("module {module_path} is ambiguous");
        }
        self.type_addr(code_slot, file_index, type_index)
    }

    fn local_symbol_type_addr(
        &self,
        code_slot: usize,
        symbol: &ServiceSymbolRef,
    ) -> anyhow::Result<TypeAddr> {
        let code = self
            .shared
            .code_slots()
            .get(code_slot)
            .with_context(|| format!("package code slot {code_slot} is out of bounds"))?;
        let mut resolved = None;
        for (file_index, file) in code.files().iter().enumerate() {
            if file.module_path != symbol.module_path {
                continue;
            }
            let declared = file
                .declarations
                .types
                .get(&symbol.symbol)
                .map(|declaration| declaration.type_index as usize);
            let linked = file
                .link_targets
                .types
                .get(&symbol.symbol)
                .map(|target| target.type_index as usize);
            for type_index in declared.into_iter().chain(linked) {
                let addr = self.type_addr(code_slot, file_index, type_index)?;
                if resolved.as_ref().is_some_and(|first| first != &addr) {
                    anyhow::bail!(
                        "type symbol {}.{} is ambiguous",
                        symbol.module_path,
                        symbol.symbol
                    );
                }
                resolved = Some(addr);
            }
        }
        resolved.with_context(|| {
            format!(
                "type symbol {}.{} is unresolved",
                symbol.module_path, symbol.symbol
            )
        })
    }

    fn service_symbol_coordinate(
        &self,
        code_slot: usize,
        symbol: &ServiceSymbolRef,
    ) -> anyhow::Result<ResolvedServiceSymbol> {
        let code = self
            .shared
            .code_slots()
            .get(code_slot)
            .with_context(|| format!("package code slot {code_slot} is out of bounds"))?;
        let mut actors = code
            .files()
            .iter()
            .filter(|file| file.module_path == symbol.module_path)
            .flat_map(|file| &file.actor_declarations)
            .filter(|declaration| declaration.abi.actor_name == symbol.symbol);
        if actors.next().is_some() {
            if actors.next().is_some() {
                anyhow::bail!(
                    "Actor type symbol {}.{} is ambiguous",
                    symbol.module_path,
                    symbol.symbol
                );
            }
            return Ok(ResolvedServiceSymbol::Actor);
        }
        self.local_symbol_type_addr(code_slot, symbol)
            .map(ResolvedServiceSymbol::Address)
    }

    fn package_symbol_type_addr(
        &self,
        caller_slot: usize,
        symbol: &skiff_artifact_model::PackageSymbolRef,
    ) -> anyhow::Result<TypeAddr> {
        let dependency_slot = self.resolve_package_ref(caller_slot, &symbol.package)?;
        let code = &self.shared.code_slots()[dependency_slot];
        if symbol
            .abi_expectation
            .as_deref()
            .is_some_and(|expected| expected != code.local_abi_identity().as_str())
        {
            anyhow::bail!("package symbol local ABI expectation mismatches linked package");
        }
        let export = code
            .artifact()
            .implementation_links
            .types
            .get(&symbol.symbol_path)
            .with_context(|| format!("package type {} is not exported", symbol.symbol_path))?;
        let file_index = self.exact_file_index(dependency_slot, &export.file)?;
        let addr = self.type_addr(dependency_slot, file_index, export.type_index as usize)?;
        let indexed = self
            .types
            .exported_package_type(dependency_slot, &symbol.symbol_path)
            .context("package type export has no canonical linked coordinate")?;
        if indexed != &addr {
            anyhow::bail!("package type export disagrees with its canonical linked coordinate");
        }
        Ok(addr)
    }

    fn resolve_package_ref(
        &self,
        caller_slot: usize,
        package_ref: &PackageRefIr,
    ) -> anyhow::Result<usize> {
        match package_ref {
            PackageRefIr::Dependency { dependency_ref } => {
                let caller =
                    self.shared.code_slots().get(caller_slot).with_context(|| {
                        format!("package code slot {caller_slot} is out of bounds")
                    })?;
                let mut matches =
                    self.shared
                        .package_link_plan()
                        .package_links
                        .iter()
                        .filter(|binding| {
                            binding.key.caller_package_build_id == *caller.package_build_id()
                                && binding.key.package_requirement_alias == *dependency_ref
                        });
                let binding = matches.next().with_context(|| {
                    format!("package dependency {dependency_ref} is unresolved")
                })?;
                if matches.next().is_some() {
                    anyhow::bail!("package dependency {dependency_ref} is ambiguous");
                }
                self.shared
                    .code_slots()
                    .iter()
                    .position(|code| code.package_build_id() == &binding.package.package_build_id)
                    .context("package dependency target is not loaded")
            }
            PackageRefIr::PackageId { package_id } => {
                let mut matches = self
                    .shared
                    .code_slots()
                    .iter()
                    .enumerate()
                    .filter(|(_, code)| code.artifact().package_id == *package_id);
                let (slot, _) = matches
                    .next()
                    .with_context(|| format!("package id {package_id} is unresolved"))?;
                if matches.next().is_some() {
                    anyhow::bail!("package id {package_id} is ambiguous in the assembly");
                }
                Ok(slot)
            }
        }
    }
}

enum ResolvedServiceSymbol {
    Address(TypeAddr),
    Actor,
}

fn validate_artifact_descriptor_matches_linked(
    coordinates: &ExactTypeCoordinateResolver<'_>,
    code_slot: usize,
    file_index: usize,
    artifact: &TypeDescriptorIr,
    linked: &LinkedTypeDescriptor,
) -> anyhow::Result<()> {
    // Rewrite only exact nominal locators. The remaining descriptor value is
    // compared in full, so equal shape or display text cannot substitute for
    // an equal linked coordinate.
    let mut resolved =
        serde_json::to_value(artifact).context("failed to encode exported type descriptor")?;
    resolve_artifact_type_coordinates(coordinates, code_slot, file_index, &mut resolved)?;
    if resolved != type_descriptor_to_value(linked) {
        anyhow::bail!("descriptor kinds or exact type coordinates differ");
    }
    Ok(())
}

fn resolve_artifact_type_coordinates(
    coordinates: &ExactTypeCoordinateResolver<'_>,
    code_slot: usize,
    file_index: usize,
    value: &mut serde_json::Value,
) -> anyhow::Result<()> {
    let kind = value
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(serde_json::Value::as_str);
    let resolved_addr = match kind {
        Some("localType") => {
            Some(coordinates.type_addr(code_slot, file_index, exact_type_index(value)?)?)
        }
        Some("publicationType") => Some(coordinates.publication_type_addr(
            code_slot,
            exact_string(value, "modulePath")?,
            exact_type_index(value)?,
        )?),
        Some("serviceSymbol") => {
            let symbol = exact_symbol(value)?;
            match coordinates.service_symbol_coordinate(code_slot, &symbol)? {
                ResolvedServiceSymbol::Address(addr) => Some(addr),
                ResolvedServiceSymbol::Actor => None,
            }
        }
        Some("packageSymbol") => {
            let symbol = serde_json::from_value(
                value
                    .get("symbol")
                    .cloned()
                    .context("package symbol coordinate has no exact symbol")?,
            )
            .context("package symbol coordinate is malformed")?;
            Some(coordinates.package_symbol_type_addr(code_slot, &symbol)?)
        }
        Some("dbObjectSymbol") => {
            Some(coordinates.local_symbol_type_addr(code_slot, &exact_symbol(value)?)?)
        }
        Some("builtin") => {
            value
                .as_object_mut()
                .expect("builtin type coordinate is an object")
                .entry("args")
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            None
        }
        Some("anyInterface") => {
            let interface = value
                .get_mut("interface")
                .and_then(serde_json::Value::as_object_mut)
                .context("interface coordinate has no exact interface")?;
            interface
                .entry("canonicalTypeArgs")
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            None
        }
        _ => None,
    };
    if let Some(addr) = resolved_addr {
        *value = serde_json::json!({
            "kind": "address",
            "addr": addr,
        });
        return Ok(());
    }
    match value {
        serde_json::Value::Object(object) => {
            for nested in object.values_mut() {
                resolve_artifact_type_coordinates(coordinates, code_slot, file_index, nested)?;
            }
        }
        serde_json::Value::Array(items) => {
            for nested in items {
                resolve_artifact_type_coordinates(coordinates, code_slot, file_index, nested)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn exact_type_index(value: &serde_json::Value) -> anyhow::Result<usize> {
    let index = value
        .get("typeIndex")
        .and_then(serde_json::Value::as_u64)
        .context("type coordinate has no exact type index")?;
    usize::try_from(index).context("type coordinate index does not fit the linked address space")
}

fn exact_string<'a>(value: &'a serde_json::Value, field: &str) -> anyhow::Result<&'a str> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("type coordinate has no exact {field}"))
}

fn exact_symbol(value: &serde_json::Value) -> anyhow::Result<ServiceSymbolRef> {
    serde_json::from_value(
        value
            .get("symbol")
            .cloned()
            .context("type coordinate has no exact symbol")?,
    )
    .context("type coordinate symbol is malformed")
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
mod tests;
