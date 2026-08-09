use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ExecutableExport, FileIrRef, InterfaceMethodSignature, NominalTypeRefBaseIr,
    OperationCallableKind, PackageArtifact, PackageCallableSignature, PackageLocalAbiSymbol,
    PackageRefIr, PackageSymbolRef, PackageTypeRef, ParamModeIr, ServiceSymbolRef,
    TypeDescriptorIr, TypeRefIr,
};

use crate::Result;

use super::{invalid_artifact, is_canonical_identifier_segment, validate_local_type_ref};

mod type_normalization;

use type_normalization::{
    instantiate_interface_methods, normalized_implementation_type,
    package_type_matches_implementation,
};

pub(super) fn validate_public_instance_surface(
    artifact: &PackageArtifact,
    public_path: &str,
    declared_receiver_type: &TypeRefIr,
    interfaces: &[TypeRefIr],
    methods: &BTreeMap<String, skiff_artifact_model::PackageCallableId>,
) -> Result<()> {
    let receiver_location = format!("public instance {public_path} receiver");
    validate_local_type_ref(declared_receiver_type, &[], &receiver_location)?;
    let Some(receiver_link) = artifact.implementation_links.constants.get(public_path) else {
        return invalid_artifact(format!(
            "public instance {public_path} has no exact receiver constant link"
        ));
    };
    validate_local_type_ref(&receiver_link.ty, &[], &receiver_location)?;
    let (receiver_symbol, receiver_type_params, receiver_target) =
        validate_public_instance_receiver(
            artifact,
            public_path,
            declared_receiver_type,
            receiver_link,
        )?;

    let mut interface_paths = BTreeSet::new();
    let mut source_type_targets = BTreeSet::from([receiver_target]);
    let mut interface_methods = BTreeMap::new();
    for interface in interfaces {
        let location = format!("public instance {public_path} interface");
        validate_local_type_ref(interface, &receiver_type_params, &location)?;
        let Some((symbol, interface_arguments)) = package_interface_instantiation(interface) else {
            return invalid_artifact(format!(
                "public instance {public_path} interface must be a canonical package interface instantiation"
            ));
        };
        let PackageRefIr::PackageId { package_id } = &symbol.package else {
            return invalid_artifact(format!(
                "public instance {public_path} interface must be owned by its package"
            ));
        };
        if package_id != &artifact.package_id || symbol.abi_expectation.is_some() {
            return invalid_artifact(format!(
                "public instance {public_path} interface {} is not a canonical current-package symbol",
                symbol.symbol_path
            ));
        }
        if !is_canonical_dotted_path(&symbol.symbol_path)
            || !interface_paths.insert(symbol.symbol_path.as_str())
        {
            return invalid_artifact(format!(
                "public instance {public_path} contains a duplicate or non-canonical interface reference {}",
                symbol.symbol_path
            ));
        }
        let resolved_interface =
            resolve_local_type_surface(artifact, &symbol.symbol_path, &location)?;
        let interface_surface = resolved_interface.surface;
        if !source_type_targets.insert(resolved_interface.source_target) {
            return invalid_artifact(format!(
                "public instance {public_path} interface {} reuses a receiver or interface source type target",
                symbol.symbol_path
            ));
        }
        if interface_surface.is_alias
            || !interface_surface.is_interface
            || !matches!(interface_surface.descriptor, TypeDescriptorIr::Interface)
        {
            return invalid_artifact(format!(
                "public instance {public_path} interface {} does not resolve to an exact interface type",
                symbol.symbol_path
            ));
        }
        let instantiated_methods = instantiate_interface_methods(
            interface_surface.interface_methods,
            interface_surface.type_params,
            interface_arguments,
        )?;
        for method in instantiated_methods {
            let method_name = method.name.clone();
            if !is_canonical_identifier_segment(&method_name) {
                return invalid_artifact(format!(
                    "public instance {public_path} interface {} contains non-canonical method name {:?}",
                    symbol.symbol_path, method_name
                ));
            }
            if interface_methods
                .insert(method_name.clone(), method)
                .is_some()
            {
                return invalid_artifact(format!(
                    "public instance {public_path} derives conflicting method {method_name} from its interfaces"
                ));
            }
        }
    }

    for method in methods.keys() {
        if !is_canonical_identifier_segment(method) {
            return invalid_artifact(format!(
                "public instance {public_path} has non-canonical method name {method:?}"
            ));
        }
    }
    let listed_method_names = methods.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let interface_method_names = interface_methods
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if listed_method_names != interface_method_names {
        return invalid_artifact(format!(
            "public instance {public_path} methods must exactly match its listed interface methods; expected {interface_method_names:?}, got {listed_method_names:?}"
        ));
    }
    for (method, callable_id) in methods {
        validate_public_instance_method_target(
            artifact,
            public_path,
            &receiver_symbol,
            &receiver_type_params,
            method,
            callable_id,
            &interface_methods[method],
        )?;
    }
    Ok(())
}

fn package_interface_instantiation(
    interface: &TypeRefIr,
) -> Option<(&PackageSymbolRef, &[TypeRefIr])> {
    match interface {
        TypeRefIr::PackageSymbol { symbol } => Some((symbol, &[])),
        TypeRefIr::AppliedNominal { base, arguments } => {
            let NominalTypeRefBaseIr::PackageSymbol { symbol } = base else {
                return None;
            };
            Some((symbol, arguments))
        }
        _ => None,
    }
}

fn validate_public_instance_receiver(
    artifact: &PackageArtifact,
    public_path: &str,
    declared_receiver_type: &TypeRefIr,
    receiver_link: &skiff_artifact_model::ConstExport,
) -> Result<(ServiceSymbolRef, Vec<String>, SourceTypeTarget)> {
    let TypeRefIr::ServiceSymbol { symbol } = declared_receiver_type else {
        return invalid_artifact(format!(
            "public instance {public_path} declared receiver must be a canonical bare service symbol"
        ));
    };
    if !is_canonical_identifier_segment(&symbol.symbol)
        || !is_canonical_dotted_path(&symbol.module_path)
    {
        return invalid_artifact(format!(
            "public instance {public_path} receiver has a non-canonical source symbol"
        ));
    }
    let Some((receiver_link_symbol, argument_count)) =
        service_symbol_nominal_root(&receiver_link.ty)
    else {
        return invalid_artifact(format!(
            "public instance {public_path} receiver constant must have an exact package-local nominal type"
        ));
    };
    if receiver_link_symbol != symbol {
        return invalid_artifact(format!(
            "public instance {public_path} declared receiver disagrees with its receiver constant nominal type"
        ));
    }
    if !artifact_has_file_owner(artifact, &receiver_link.file) {
        return invalid_artifact(format!(
            "public instance {public_path} receiver constant file is absent from package files"
        ));
    }
    if !is_canonical_identifier_segment(&receiver_link.symbol)
        || !is_canonical_dotted_path(&receiver_link.file.module_path)
    {
        return invalid_artifact(format!(
            "public instance {public_path} receiver constant link has non-canonical provenance"
        ));
    }
    let source_const_path = format!(
        "{}.{}",
        receiver_link.file.module_path, receiver_link.symbol
    );
    let Some(PackageLocalAbiSymbol::Constant {
        ty: source_const_type,
        ..
    }) = artifact
        .package_local_abi
        .implementation_symbols
        .get(&source_const_path)
    else {
        return invalid_artifact(format!(
            "public instance {public_path} receiver has no source constant {source_const_path}"
        ));
    };
    let PackageTypeRef::Local {
        local_type: source_const_local_type,
    } = source_const_type
    else {
        return invalid_artifact(format!(
            "public instance {public_path} source constant {source_const_path} must have an exact local receiver type"
        ));
    };
    if normalized_implementation_type(artifact, &receiver_link.ty, None)?
        != normalized_implementation_type(artifact, source_const_local_type, None)?
    {
        return invalid_artifact(format!(
            "public instance {public_path} source constant {source_const_path} disagrees with its declared receiver arguments"
        ));
    }
    let Some(source_const_link) = artifact
        .implementation_links
        .constants
        .get(&source_const_path)
    else {
        return invalid_artifact(format!(
            "public instance {public_path} receiver has no source constant link {source_const_path}"
        ));
    };
    if source_const_link.file != receiver_link.file
        || source_const_link.const_index != receiver_link.const_index
        || (source_const_link.symbol != source_const_path
            && !(source_const_path == public_path
                && source_const_link.symbol == receiver_link.symbol))
    {
        return invalid_artifact(format!(
            "public instance {public_path} receiver constant link disagrees with source constant {source_const_path}"
        ));
    }
    if normalized_implementation_type(artifact, &source_const_link.ty, None)?
        != normalized_implementation_type(artifact, source_const_local_type, None)?
    {
        return invalid_artifact(format!(
            "public instance {public_path} source constant link {source_const_path} has mismatched receiver arguments"
        ));
    }

    let source_path = symbol.symbol_path();
    let Some(PackageLocalAbiSymbol::Type {
        descriptor,
        is_alias,
        is_interface,
        type_params,
        interface_methods,
        ..
    }) = artifact
        .package_local_abi
        .implementation_symbols
        .get(&source_path)
    else {
        return invalid_artifact(format!(
            "public instance {public_path} receiver type {source_path} has no exact implementation type"
        ));
    };
    if *is_alias
        || *is_interface
        || matches!(
            descriptor,
            TypeDescriptorIr::Alias { .. } | TypeDescriptorIr::Interface
        )
        || !interface_methods.is_empty()
    {
        return invalid_artifact(format!(
            "public instance {public_path} receiver type {source_path} is not a concrete nominal type"
        ));
    }
    if type_params.len() != argument_count {
        return invalid_artifact(format!(
            "public instance {public_path} receiver type {source_path} has {} type arguments, expected {}",
            argument_count,
            type_params.len()
        ));
    }
    let Some(type_link) = artifact.implementation_links.types.get(&source_path) else {
        return invalid_artifact(format!(
            "public instance {public_path} receiver type {source_path} has no exact implementation link"
        ));
    };
    if !artifact_has_file_owner(artifact, &type_link.file)
        || type_link.file.module_path != symbol.module_path
        || (type_link.symbol != symbol.symbol && type_link.symbol != source_path)
        || type_link.is_interface
        || type_link.descriptor.as_ref() != Some(descriptor)
        || type_link.type_params != *type_params
        || type_link.interface_methods != *interface_methods
    {
        return invalid_artifact(format!(
            "public instance {public_path} receiver type {source_path} disagrees with its implementation link"
        ));
    }
    Ok((
        symbol.clone(),
        type_params.clone(),
        SourceTypeTarget::from_type_export(type_link),
    ))
}

fn validate_public_instance_method_target(
    artifact: &PackageArtifact,
    public_path: &str,
    receiver: &ServiceSymbolRef,
    receiver_type_params: &[String],
    method: &str,
    callable_id: &skiff_artifact_model::PackageCallableId,
    interface_method: &InterfaceMethodSignature,
) -> Result<()> {
    let method_public_path = format!("{public_path}.{method}");
    let Some(PackageLocalAbiSymbol::Callable {
        callable_id: public_callable_id,
        signature: public_signature,
    }) = artifact
        .package_local_abi
        .public_symbols
        .get(&method_public_path)
    else {
        return invalid_artifact(format!(
            "public instance {public_path} method {method} has no exact public callable"
        ));
    };
    if public_callable_id != callable_id {
        return invalid_artifact(format!(
            "public instance {public_path} method {method} disagrees with public callable {method_public_path}"
        ));
    }
    let Some(callable_link) = artifact.callable_links.get(callable_id) else {
        return invalid_artifact(format!(
            "public instance {public_path} method {method} has no exact callable link"
        ));
    };
    if callable_link.target.callable_kind != OperationCallableKind::ImplMethod {
        return invalid_artifact(format!(
            "public instance {public_path} method {method} callable target is not an implementation method"
        ));
    }
    let mut method_links = artifact
        .implementation_links
        .impl_methods
        .iter()
        .filter(|(_, link)| {
            link.file == callable_link.target.file_ref
                && link.executable_index == callable_link.target.executable_index
        });
    let Some((implementation_path, method_link)) = method_links.next() else {
        return invalid_artifact(format!(
            "public instance {public_path} method {method} has no exact receiver implementation link"
        ));
    };
    if method_links.next().is_some()
        || !artifact_has_file_owner(artifact, &method_link.file)
        || method_link.file.module_path != receiver.module_path
        || method_link.symbol != *implementation_path
        || !implementation_method_symbol_matches_receiver(
            implementation_path,
            receiver,
            receiver_type_params,
            method,
        )
    {
        return invalid_artifact(format!(
            "public instance {public_path} method {method} has non-canonical receiver implementation provenance"
        ));
    }
    validate_public_instance_method_signature(
        artifact,
        public_path,
        receiver,
        receiver_type_params,
        interface_method,
        method_link,
        public_signature,
    )?;
    Ok(())
}

fn implementation_method_symbol_matches_receiver(
    implementation_path: &str,
    receiver: &ServiceSymbolRef,
    receiver_type_params: &[String],
    method: &str,
) -> bool {
    let Some(owner) = implementation_path.strip_suffix(&format!(".{method}")) else {
        return false;
    };
    let (base, arguments) = match owner.rsplit_once('<') {
        Some((base, arguments)) if arguments.ends_with('>') => {
            (base, Some(&arguments[..arguments.len() - 1]))
        }
        Some(_) => return false,
        None => (owner, None),
    };
    let arguments = arguments
        .map(|arguments| arguments.split(',').map(str::trim).collect::<Vec<_>>())
        .unwrap_or_default();
    if arguments
        != receiver_type_params
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    {
        return false;
    }
    [
        receiver.symbol.clone(),
        format!("{}.{}", receiver.module_path, receiver.symbol),
        format!("root.{}.{}", receiver.module_path, receiver.symbol),
    ]
    .iter()
    .any(|candidate| base == candidate)
}

fn validate_public_instance_method_signature(
    artifact: &PackageArtifact,
    public_path: &str,
    receiver: &ServiceSymbolRef,
    receiver_type_params: &[String],
    interface_method: &InterfaceMethodSignature,
    method_link: &ExecutableExport,
    public_signature: &PackageCallableSignature,
) -> Result<()> {
    if interface_method.is_static
        || interface_method.is_native
        || interface_method.is_provider
        || !interface_method.type_params.is_empty()
    {
        return invalid_artifact(format!(
            "public instance {public_path} method {} has unsupported interface flags or method type parameters",
            interface_method.name
        ));
    }
    if public_signature.type_params != receiver_type_params {
        return invalid_artifact(format!(
            "public instance {public_path} method {} callable type parameters do not match its receiver declaration",
            interface_method.name
        ));
    }
    let explicit_self = method_link
        .signature
        .params
        .first()
        .filter(|parameter| parameter.name == "self");
    let Some(implementation_receiver_type) = method_link.signature.self_type.as_ref() else {
        return invalid_artifact(format!(
            "public instance {public_path} method {} implementation has no receiver type",
            interface_method.name
        ));
    };
    if let Some(self_param) = explicit_self {
        if self_param.mode != ParamModeIr::Value || &self_param.ty != implementation_receiver_type {
            return invalid_artifact(format!(
                "public instance {public_path} method {} explicit implementation receiver does not exactly match selfType",
                interface_method.name
            ));
        }
    }
    let implementation_params =
        &method_link.signature.params[usize::from(explicit_self.is_some())..];
    if implementation_params
        .iter()
        .any(|parameter| parameter.name == "self")
    {
        return invalid_artifact(format!(
            "public instance {public_path} method {} implementation has a non-leading receiver",
            interface_method.name
        ));
    }
    let expected_receiver_type = receiver_definition_type(receiver, receiver_type_params);
    if normalized_implementation_type(artifact, implementation_receiver_type, None)?
        != normalized_implementation_type(artifact, &expected_receiver_type, None)?
    {
        return invalid_artifact(format!(
            "public instance {public_path} method {} implementation receiver does not match the exact receiver declaration",
            interface_method.name
        ));
    }

    let normalized_self =
        normalized_implementation_type(artifact, implementation_receiver_type, None)?;
    let explicit_interface_self = interface_method
        .params
        .first()
        .filter(|parameter| parameter.name == "self");
    let interface_params = match (
        explicit_interface_self,
        interface_method.implicit_self.as_ref(),
    ) {
        (Some(_), Some(_)) => {
            return invalid_artifact(format!(
                "public instance {public_path} interface method {} declares two receivers",
                interface_method.name
            ));
        }
        (Some(parameter), None)
            if matches!(
                &parameter.ty,
                TypeRefIr::Builtin { name, args } if name == "Self" && args.is_empty()
            ) =>
        {
            &interface_method.params[1..]
        }
        (Some(_), None) => {
            return invalid_artifact(format!(
                "public instance {public_path} interface method {} has a non-canonical explicit receiver",
                interface_method.name
            ));
        }
        (None, Some(interface_receiver))
            if normalized_implementation_type(
                artifact,
                interface_receiver,
                Some(&normalized_self),
            )? == normalized_self =>
        {
            interface_method.params.as_slice()
        }
        (None, Some(_)) => {
            return invalid_artifact(format!(
                "public instance {public_path} interface method {} has a mismatched implicit receiver",
                interface_method.name
            ));
        }
        (None, None) => {
            return invalid_artifact(format!(
                "public instance {public_path} interface method {} has no receiver parameter",
                interface_method.name
            ));
        }
    };
    if interface_params.len() != implementation_params.len()
        || interface_params.len() != public_signature.parameters.len()
    {
        return invalid_artifact(format!(
            "public instance {public_path} method {} parameter count disagrees with its interface",
            interface_method.name
        ));
    }
    for ((interface_param, implementation_param), public_param) in interface_params
        .iter()
        .zip(implementation_params)
        .zip(&public_signature.parameters)
    {
        if interface_param.name != implementation_param.name
            || interface_param.name != public_param.name
            || normalized_implementation_type(
                artifact,
                &interface_param.ty,
                Some(&normalized_self),
            )? != normalized_implementation_type(artifact, &implementation_param.ty, None)?
            || !package_type_matches_implementation(
                artifact,
                &public_param.ty,
                &interface_param.ty,
                Some(&normalized_self),
            )?
        {
            return invalid_artifact(format!(
                "public instance {public_path} method {} parameter {} disagrees with its interface",
                interface_method.name, interface_param.name
            ));
        }
    }
    if normalized_implementation_type(
        artifact,
        &interface_method.return_type,
        Some(&normalized_self),
    )? != normalized_implementation_type(artifact, &method_link.signature.return_type, None)?
        || !package_type_matches_implementation(
            artifact,
            &public_signature.return_type,
            &interface_method.return_type,
            Some(&normalized_self),
        )?
        || public_signature.may_suspend != method_link.signature.may_suspend
    {
        return invalid_artifact(format!(
            "public instance {public_path} method {} return shape or concrete suspension summary is inconsistent",
            interface_method.name
        ));
    }
    Ok(())
}

fn receiver_definition_type(
    receiver: &ServiceSymbolRef,
    receiver_type_params: &[String],
) -> TypeRefIr {
    if receiver_type_params.is_empty() {
        return TypeRefIr::ServiceSymbol {
            symbol: receiver.clone(),
        };
    }
    TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::ServiceSymbol {
            symbol: receiver.clone(),
        },
        arguments: receiver_type_params
            .iter()
            .map(|name| TypeRefIr::TypeParam { name: name.clone() })
            .collect(),
    }
}

fn service_symbol_nominal_root(ty: &TypeRefIr) -> Option<(&ServiceSymbolRef, usize)> {
    match ty {
        TypeRefIr::ServiceSymbol { symbol } => Some((symbol, 0)),
        TypeRefIr::AppliedNominal { base, arguments } => {
            let NominalTypeRefBaseIr::ServiceSymbol { symbol } = base else {
                return None;
            };
            Some((symbol, arguments.len()))
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct LocalTypeSurface<'a> {
    descriptor: &'a TypeDescriptorIr,
    is_alias: bool,
    is_interface: bool,
    type_params: &'a [String],
    interface_methods: &'a [InterfaceMethodSignature],
}

struct ResolvedLocalTypeSurface<'a> {
    surface: LocalTypeSurface<'a>,
    source_target: SourceTypeTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SourceTypeTarget {
    file_ir_identity: String,
    module_path: String,
    type_index: u32,
}

impl SourceTypeTarget {
    fn from_type_export(export: &skiff_artifact_model::TypeExport) -> Self {
        Self {
            file_ir_identity: export.file.file_ir_identity.clone(),
            module_path: export.file.module_path.clone(),
            type_index: export.type_index,
        }
    }
}

fn resolve_local_type_surface<'a>(
    artifact: &'a PackageArtifact,
    symbol_path: &str,
    location: &str,
) -> Result<ResolvedLocalTypeSurface<'a>> {
    let public = artifact
        .package_local_abi
        .public_symbols
        .get(symbol_path)
        .and_then(local_type_surface);
    let implementation = artifact
        .package_local_abi
        .implementation_symbols
        .get(symbol_path)
        .and_then(local_type_surface);
    let surface = match (public, implementation) {
        (None, None) => invalid_artifact(format!(
            "{location} {symbol_path} does not resolve to a package type"
        ))?,
        (Some(surface), None) | (None, Some(surface)) => surface,
        (Some(public), Some(implementation))
            if local_type_surfaces_are_compatible(public, implementation) =>
        {
            public
        }
        (Some(_), Some(_)) => invalid_artifact(format!(
            "{location} {symbol_path} resolves to conflicting public and implementation types"
        ))?,
    };
    let Some(link) = artifact.implementation_links.types.get(symbol_path) else {
        return invalid_artifact(format!(
            "{location} {symbol_path} has no exact implementation type link"
        ));
    };
    if !artifact_has_file_owner(artifact, &link.file)
        || link.descriptor.as_ref() != Some(surface.descriptor)
        || link.is_interface != surface.is_interface
        || link.type_params != surface.type_params
        || link.interface_methods != surface.interface_methods
    {
        return invalid_artifact(format!(
            "{location} {symbol_path} disagrees with its implementation type link"
        ));
    }
    let source_path = match (public, implementation) {
        (None, Some(_)) => {
            let Some((module_path, name)) = split_canonical_source_path(symbol_path) else {
                return invalid_artifact(format!(
                    "{location} {symbol_path} is not a canonical implementation type path"
                ));
            };
            if link.file.module_path != module_path || link.symbol != symbol_path {
                return invalid_artifact(format!(
                    "{location} {symbol_path} has non-canonical implementation type provenance"
                ));
            }
            format!("{module_path}.{name}")
        }
        (Some(_), None) => {
            if !is_canonical_identifier_segment(&link.symbol)
                || !is_canonical_dotted_path(&link.file.module_path)
            {
                return invalid_artifact(format!(
                    "{location} {symbol_path} has non-canonical public type provenance"
                ));
            }
            format!("{}.{}", link.file.module_path, link.symbol)
        }
        (Some(_), Some(_)) => {
            if !is_canonical_identifier_segment(&link.symbol)
                || symbol_path != format!("{}.{}", link.file.module_path, link.symbol)
            {
                return invalid_artifact(format!(
                    "{location} {symbol_path} has non-canonical shared public/source type provenance"
                ));
            }
            symbol_path.to_string()
        }
        (None, None) => unreachable!("missing surfaces returned above"),
    };
    let Some(source_surface) = artifact
        .package_local_abi
        .implementation_symbols
        .get(&source_path)
        .and_then(local_type_surface)
    else {
        return invalid_artifact(format!(
            "{location} {symbol_path} has no exact source interface {source_path}"
        ));
    };
    if !local_type_surfaces_are_compatible(surface, source_surface) {
        return invalid_artifact(format!(
            "{location} {symbol_path} disagrees with source interface {source_path}"
        ));
    }
    let Some(source_link) = artifact.implementation_links.types.get(&source_path) else {
        return invalid_artifact(format!(
            "{location} {symbol_path} has no source interface link {source_path}"
        ));
    };
    let source_target = SourceTypeTarget::from_type_export(source_link);
    if source_target != SourceTypeTarget::from_type_export(link)
        || !artifact_has_file_owner(artifact, &source_link.file)
        || source_link.descriptor.as_ref() != Some(source_surface.descriptor)
        || source_link.is_interface != source_surface.is_interface
        || source_link.type_params != source_surface.type_params
        || source_link.interface_methods != source_surface.interface_methods
        || !source_link_symbol_matches_path(source_link, &source_path)
    {
        return invalid_artifact(format!(
            "{location} {symbol_path} disagrees with source interface link {source_path}"
        ));
    }
    Ok(ResolvedLocalTypeSurface {
        surface,
        source_target,
    })
}

fn local_type_surface(symbol: &PackageLocalAbiSymbol) -> Option<LocalTypeSurface<'_>> {
    let PackageLocalAbiSymbol::Type {
        descriptor,
        is_alias,
        is_interface,
        type_params,
        interface_methods,
        ..
    } = symbol
    else {
        return None;
    };
    Some(LocalTypeSurface {
        descriptor,
        is_alias: *is_alias,
        is_interface: *is_interface,
        type_params,
        interface_methods,
    })
}

fn local_type_surfaces_are_compatible(
    left: LocalTypeSurface<'_>,
    right: LocalTypeSurface<'_>,
) -> bool {
    left.descriptor == right.descriptor
        && left.is_alias == right.is_alias
        && left.is_interface == right.is_interface
        && left.type_params == right.type_params
        && left.interface_methods == right.interface_methods
}

fn split_canonical_source_path(path: &str) -> Option<(&str, &str)> {
    let (module_path, name) = path.rsplit_once('.')?;
    (is_canonical_dotted_path(module_path) && is_canonical_identifier_segment(name))
        .then_some((module_path, name))
}

fn source_link_symbol_matches_path(
    link: &skiff_artifact_model::TypeExport,
    source_path: &str,
) -> bool {
    split_canonical_source_path(source_path).is_some_and(|(module_path, name)| {
        link.file.module_path == module_path && (link.symbol == source_path || link.symbol == name)
    })
}

fn is_canonical_dotted_path(path: &str) -> bool {
    !path.is_empty() && path.split('.').all(is_canonical_identifier_segment)
}

fn artifact_has_file_owner(artifact: &PackageArtifact, file: &FileIrRef) -> bool {
    artifact.files.iter().any(|candidate| {
        candidate.file_ir_identity == file.file_ir_identity
            && candidate.module_path == file.module_path
    })
}
