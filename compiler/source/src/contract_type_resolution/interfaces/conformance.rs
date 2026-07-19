use std::collections::BTreeMap;

use skiff_artifact_model::PackageTypeRef;

use crate::{
    parsed_sources::ParsedCompilerSource,
    shared::{
        ast::{FunctionDecl, InterfaceDecl, InterfaceOperation, TypeRef},
        type_expr::TypeExpr,
    },
    type_resolution_model::CanonicalInterfaceOwnerResolution,
    SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

use super::{
    substitution::substitute_requirement, ContractAwareTypeResolver, ImplementationMethod,
    SourceInterfaceDecl,
};
use crate::contract_type_resolution::{
    SourceExecutableReceiver, SourceExecutableSignature, SourceExecutableSignatureFacts,
    SourceInterfaceConformanceKey, SourceInterfaceMethodKey, SourceInterfaceRequirementSignature,
    ValidatedSourceInterfaceConformance, ValidatedSourceInterfaceMethod,
};

pub(super) struct ConformanceInput<'a> {
    pub parsed_sources: &'a [ParsedCompilerSource],
    pub type_resolution: &'a TypeResolutionModel,
    pub resolver: &'a ContractAwareTypeResolver<'a>,
    pub interfaces: &'a BTreeMap<SourceSymbolKey, SourceInterfaceDecl<'a>>,
    pub requirements: &'a BTreeMap<SourceInterfaceMethodKey, SourceInterfaceRequirementSignature>,
    pub implementations: &'a BTreeMap<(SourceSymbolKey, String), ImplementationMethod<'a>>,
    pub executable_signatures: &'a SourceExecutableSignatureFacts,
}

pub(super) fn build_conformances(
    input: ConformanceInput<'_>,
) -> Result<BTreeMap<SourceInterfaceConformanceKey, ValidatedSourceInterfaceConformance>, String> {
    let mut conformances = BTreeMap::new();
    for parsed in input.parsed_sources {
        for ty in &parsed.ast().types {
            if ty.alias.is_some() {
                continue;
            }
            let receiver = SourceSymbolKey::new(parsed.module_path(), &ty.name);
            let context = TypeResolutionContext::with_type_params(
                parsed.module_path(),
                ty.type_params.iter().cloned().collect(),
            );
            let receiver_type_text = if ty.type_params.is_empty() {
                ty.name.clone()
            } else {
                format!("{}<{}>", ty.name, ty.type_params.join(", "))
            };
            let declared_receiver_type = input.resolver.resolve_source_type_ref(
                &TypeRef {
                    name: receiver_type_text,
                },
                &context,
            )?;
            for implemented in &ty.implements {
                let Some((interface_key, interface_arguments)) = resolve_interface_instantiation(
                    implemented,
                    &context,
                    input.type_resolution,
                    input.resolver,
                )?
                else {
                    continue;
                };
                let interface = input.interfaces.get(&interface_key).ok_or_else(|| {
                    format!(
                        "type `{receiver}` implements `{}`, which has no source-owned exact interface fact",
                        implemented.name
                    )
                })?;
                if interface.declaration.type_params.len() != interface_arguments.len() {
                    return Err(format!(
                        "interface `{interface_key}` expects {} type arguments, found {}",
                        interface.declaration.type_params.len(),
                        interface_arguments.len()
                    ));
                }
                let key = SourceInterfaceConformanceKey {
                    receiver: receiver.clone(),
                    interface: interface_key.clone(),
                };
                let substitutions = interface
                    .declaration
                    .type_params
                    .iter()
                    .cloned()
                    .zip(interface_arguments.iter().cloned())
                    .collect::<BTreeMap<_, _>>();
                let validated = validate_conformance(
                    &input,
                    &key,
                    &declared_receiver_type,
                    &interface_arguments,
                    substitutions,
                    interface.declaration,
                )?;
                if conformances.insert(key.clone(), validated).is_some() {
                    return Err(format!(
                        "type `{receiver}` declares conformance to `{interface_key}` more than once"
                    ));
                }
            }
        }
    }
    Ok(conformances)
}

fn resolve_interface_instantiation(
    implemented: &TypeRef,
    context: &TypeResolutionContext<'_>,
    type_resolution: &TypeResolutionModel,
    resolver: &ContractAwareTypeResolver<'_>,
) -> Result<Option<(SourceSymbolKey, Vec<PackageTypeRef>)>, String> {
    let interface =
        match type_resolution.classify_canonical_interface_owner(&implemented.name, context) {
            CanonicalInterfaceOwnerResolution::SourceDeclaredExact { interface, .. } => interface,
            CanonicalInterfaceOwnerResolution::TypedPackage { .. }
            | CanonicalInterfaceOwnerResolution::CompilerKnown { .. } => return Ok(None),
            CanonicalInterfaceOwnerResolution::InvalidOrUnresolved { message } => {
                return Err(message);
            }
        };
    let TypeExpr::Named { args, .. } = TypeExpr::parse(&implemented.name) else {
        return Err(format!(
            "implements entry `{}` is not a named interface",
            implemented.name
        ));
    };
    let arguments = args
        .iter()
        .map(|argument| {
            resolver.resolve_source_type_ref(
                &TypeRef {
                    name: argument.to_type_string(),
                },
                context,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some((interface, arguments)))
}

fn validate_conformance(
    input: &ConformanceInput<'_>,
    key: &SourceInterfaceConformanceKey,
    declared_receiver_type: &PackageTypeRef,
    interface_arguments: &[PackageTypeRef],
    mut substitutions: BTreeMap<String, PackageTypeRef>,
    interface: &InterfaceDecl,
) -> Result<ValidatedSourceInterfaceConformance, String> {
    let mut methods = BTreeMap::new();
    for operation in &interface.operations {
        let requirement_key = SourceInterfaceMethodKey {
            interface: key.interface.clone(),
            method: operation.name.clone(),
        };
        let requirement = input.requirements.get(&requirement_key).ok_or_else(|| {
            format!(
                "interface `{}.{}` has no exact requirement fact",
                key.interface, operation.name
            )
        })?;
        let implementation = input
            .implementations
            .get(&(key.receiver.clone(), operation.name.clone()))
            .ok_or_else(|| {
                format!(
                    "type `{}` declares conformance to `{}`, but method `{}` is missing",
                    key.receiver, key.interface, operation.name
                )
            })?;
        let executable = input
            .executable_signatures
            .signature(&implementation.source_key)
            .ok_or_else(|| {
                format!(
                    "interface implementation method `{}` has no exact executable signature fact",
                    implementation.source_key
                )
            })?;
        validate_method_flags(key, operation, implementation.declaration)?;
        let receiver_type = executable_receiver_type(executable).ok_or_else(|| {
            format!(
                "type `{}` method `{}` has no instance receiver",
                key.receiver, operation.name
            )
        })?;
        if &receiver_type != declared_receiver_type {
            return Err(format!(
                "type `{}` method `{}` receiver does not match its declared receiver type",
                key.receiver, operation.name
            ));
        }
        substitutions.insert("Self".to_string(), declared_receiver_type.clone());
        let exact_requirement = substitute_requirement(requirement, &substitutions)?;
        validate_receiver_and_signature(key, operation, &exact_requirement, executable)?;
        methods.insert(
            operation.name.clone(),
            ValidatedSourceInterfaceMethod {
                key: requirement_key,
                exact_requirement,
                executable: implementation.source_key.clone(),
                receiver_type: declared_receiver_type.clone(),
            },
        );
    }
    Ok(ValidatedSourceInterfaceConformance {
        key: key.clone(),
        interface_arguments: interface_arguments.to_vec(),
        canonical_substitutions: substitutions,
        methods,
    })
}

fn validate_method_flags(
    key: &SourceInterfaceConformanceKey,
    requirement: &InterfaceOperation,
    implementation: &FunctionDecl,
) -> Result<(), String> {
    if !implementation.type_params.is_empty() {
        return Err(format!(
            "type `{}` method `{}` cannot satisfy interface `{}` because method-level type parameters are unsupported",
            key.receiver, implementation.name, key.interface
        ));
    }
    if implementation.is_static != requirement.is_static
        || implementation.is_native != requirement.is_native
        || implementation.is_provider != requirement.is_provider
    {
        return Err(format!(
            "type `{}` method `{}` flags do not match interface `{}`",
            key.receiver, implementation.name, key.interface
        ));
    }
    Ok(())
}

fn executable_receiver_type(executable: &SourceExecutableSignature) -> Option<PackageTypeRef> {
    match &executable.receiver {
        SourceExecutableReceiver::Implicit { ty } => Some(ty.clone()),
        SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 } => executable
            .parameters
            .first()
            .map(|parameter| parameter.ty.clone()),
        SourceExecutableReceiver::None
        | SourceExecutableReceiver::ExplicitParameter { parameter_index: _ } => None,
    }
}

fn validate_receiver_and_signature(
    key: &SourceInterfaceConformanceKey,
    operation: &InterfaceOperation,
    requirement: &SourceInterfaceRequirementSignature,
    executable: &SourceExecutableSignature,
) -> Result<(), String> {
    let actual_parameters = match (&requirement.receiver, &executable.receiver) {
        (SourceExecutableReceiver::Implicit { .. }, SourceExecutableReceiver::Implicit { .. }) => {
            executable.parameters.as_slice()
        }
        (
            SourceExecutableReceiver::Implicit { .. },
            SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 },
        ) => executable.parameters.get(1..).ok_or_else(|| {
            format!(
                "type `{}` method `{}` explicit receiver is missing",
                key.receiver, operation.name
            )
        })?,
        (
            SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 },
            SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 },
        ) => executable.parameters.as_slice(),
        (
            SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 },
            SourceExecutableReceiver::Implicit { .. },
        ) => {
            let Some((expected_receiver, expected_parameters)) =
                requirement.parameters.split_first()
            else {
                return Err(format!(
                    "interface `{}.{}` explicit receiver requirement is missing its receiver parameter",
                    key.interface, operation.name
                ));
            };
            let SourceExecutableReceiver::Implicit {
                ty: actual_receiver,
            } = &executable.receiver
            else {
                unreachable!("match arm requires an implicit receiver");
            };
            if &expected_receiver.ty != actual_receiver {
                return Err(format!(
                    "type `{}` method `{}` receiver does not match interface `{}`",
                    key.receiver, operation.name, key.interface
                ));
            }
            return validate_parameter_and_return_types(
                key,
                operation,
                expected_parameters,
                executable.parameters.as_slice(),
                &requirement.return_type,
                &executable.return_type,
            );
        }
        _ => {
            return Err(format!(
                "type `{}` method `{}` receiver does not match interface `{}`",
                key.receiver, operation.name, key.interface
            ));
        }
    };
    validate_parameter_and_return_types(
        key,
        operation,
        &requirement.parameters,
        actual_parameters,
        &requirement.return_type,
        &executable.return_type,
    )
}

fn validate_parameter_and_return_types(
    key: &SourceInterfaceConformanceKey,
    operation: &InterfaceOperation,
    expected_parameters: &[skiff_artifact_model::PackageCallableParameter],
    actual_parameters: &[skiff_artifact_model::PackageCallableParameter],
    expected_return: &PackageTypeRef,
    actual_return: &PackageTypeRef,
) -> Result<(), String> {
    let parameter_types_match = actual_parameters.len() == expected_parameters.len()
        && actual_parameters
            .iter()
            .zip(expected_parameters)
            .all(|(actual, expected)| actual.ty == expected.ty);
    if !parameter_types_match || actual_return != expected_return {
        return Err(format!(
            "type `{}` method `{}` exact signature does not match interface `{}`; expected params {:?} return {:?}, got params {:?} return {:?}",
            key.receiver,
            operation.name,
            key.interface,
            expected_parameters.iter().map(|parameter| &parameter.ty).collect::<Vec<_>>(),
            expected_return,
            actual_parameters.iter().map(|parameter| &parameter.ty).collect::<Vec<_>>(),
            actual_return,
        ));
    }
    Ok(())
}
