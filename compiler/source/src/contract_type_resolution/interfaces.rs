use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::PackageCallableParameter;

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{
        ast::{FunctionDecl, InterfaceDecl, InterfaceOperation, TypeRef},
        type_expr::TypeExpr,
    },
    SourceDependencyAnalysisInput, SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

use super::{
    types::ContractAwareTypeResolver, SourceExecutableReceiver, SourceExecutableSignatureFacts,
    SourceInterfaceMethodKey, SourceInterfaceRequirementSignature, SourceInterfaceSignatureFacts,
};

mod conformance;
mod substitution;

use conformance::{build_conformances, ConformanceInput};
pub(crate) use substitution::substitute_package_type;

pub(super) fn build_interface_signature_facts(
    parsed_sources: &[ParsedCompilerSource],
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
    executable_signatures: &SourceExecutableSignatureFacts,
) -> Result<SourceInterfaceSignatureFacts, String> {
    let resolver = ContractAwareTypeResolver::new(type_resolution, dependency_analysis);
    let interfaces = index_interfaces(parsed_sources)?;
    let requirements = build_requirements(&interfaces, &resolver)?;
    let implementations = index_implementation_methods(parsed_sources, type_resolution)?;
    let conformances = build_conformances(ConformanceInput {
        parsed_sources,
        type_resolution,
        resolver: &resolver,
        interfaces: &interfaces,
        requirements: &requirements,
        implementations: &implementations,
        executable_signatures,
    })?;
    Ok(SourceInterfaceSignatureFacts {
        requirements,
        conformances,
    })
}

pub(super) struct SourceInterfaceDecl<'a> {
    module_path: &'a str,
    declaration: &'a InterfaceDecl,
}

fn index_interfaces<'a>(
    parsed_sources: &'a [ParsedCompilerSource],
) -> Result<BTreeMap<SourceSymbolKey, SourceInterfaceDecl<'a>>, String> {
    let mut interfaces = BTreeMap::new();
    for parsed in parsed_sources {
        for declaration in &parsed.ast().interfaces {
            let key = SourceSymbolKey::new(parsed.module_path(), &declaration.name);
            if interfaces
                .insert(
                    key.clone(),
                    SourceInterfaceDecl {
                        module_path: parsed.module_path(),
                        declaration,
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "source interface `{key}` is declared more than once"
                ));
            }
        }
    }
    Ok(interfaces)
}

fn build_requirements(
    interfaces: &BTreeMap<SourceSymbolKey, SourceInterfaceDecl<'_>>,
    resolver: &ContractAwareTypeResolver<'_>,
) -> Result<BTreeMap<SourceInterfaceMethodKey, SourceInterfaceRequirementSignature>, String> {
    let mut requirements = BTreeMap::new();
    for (interface_key, source) in interfaces {
        let declaration = source.declaration;
        for operation in &declaration.operations {
            let key = SourceInterfaceMethodKey {
                interface: interface_key.clone(),
                method: operation.name.clone(),
            };
            let signature =
                exact_requirement_signature(source.module_path, declaration, operation, resolver)
                    .map_err(|error| {
                    format!(
                        "interface requirement `{}.{}`: {error}",
                        interface_key, operation.name
                    )
                })?;
            if requirements.insert(key.clone(), signature).is_some() {
                return Err(format!(
                    "interface requirement `{}.{}` is declared more than once",
                    key.interface, key.method
                ));
            }
        }
    }
    Ok(requirements)
}

fn exact_requirement_signature(
    module_path: &str,
    interface: &InterfaceDecl,
    operation: &InterfaceOperation,
    resolver: &ContractAwareTypeResolver<'_>,
) -> Result<SourceInterfaceRequirementSignature, String> {
    if !operation.type_params.is_empty() {
        return Err(
            "method-level type parameters are not supported in exact interface requirements"
                .to_string(),
        );
    }
    if operation.is_static || operation.is_native || operation.is_provider {
        return Err("static/native/provider interface requirements are not supported".to_string());
    }
    let mut type_params = interface
        .type_params
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    type_params.insert("Self".to_string());
    let context = TypeResolutionContext::with_type_params(module_path, type_params);
    let parameters = operation
        .params
        .iter()
        .map(|parameter| {
            Ok(PackageCallableParameter {
                name: parameter.name.clone(),
                ty: resolver.resolve_source_type_ref(&parameter.ty, &context)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let return_type = resolver.resolve_source_type_ref(&operation.return_type, &context)?;
    let receiver = if parameters
        .first()
        .is_some_and(|parameter| parameter.name == "self")
    {
        SourceExecutableReceiver::ExplicitParameter { parameter_index: 0 }
    } else {
        SourceExecutableReceiver::Implicit {
            ty: resolver.resolve_source_type_ref(
                &TypeRef {
                    name: "Self".to_string(),
                },
                &context,
            )?,
        }
    };
    Ok(SourceInterfaceRequirementSignature {
        parameters,
        return_type,
        receiver,
        interface_type_params: interface.type_params.clone(),
        method_type_params: operation.type_params.clone(),
        is_native: operation.is_native,
        is_provider: operation.is_provider,
        is_static: operation.is_static,
    })
}

pub(super) struct ImplementationMethod<'a> {
    source_key: SourceSymbolKey,
    declaration: &'a FunctionDecl,
}

fn index_implementation_methods<'a>(
    parsed_sources: &'a [ParsedCompilerSource],
    type_resolution: &TypeResolutionModel,
) -> Result<BTreeMap<(SourceSymbolKey, String), ImplementationMethod<'a>>, String> {
    let mut methods = BTreeMap::new();
    for parsed in parsed_sources {
        for implementation in &parsed.ast().impls {
            let context = TypeResolutionContext::source(parsed.module_path());
            let TypeExpr::Named { name, .. } = TypeExpr::parse(&implementation.target) else {
                return Err(format!(
                    "impl target `{}` is not a named source type",
                    implementation.target
                ));
            };
            let receiver = type_resolution
                .resolve_source_type_key(&name, &context)
                .ok_or_else(|| {
                    format!(
                        "impl target `{}` does not resolve to a source type",
                        implementation.target
                    )
                })?;
            for method in &implementation.method_bodies {
                let map_key = (receiver.clone(), method.name.clone());
                let source_key = SourceSymbolKey::new(
                    parsed.module_path(),
                    impl_method_declaration_name(&implementation.target, &method.name),
                );
                if methods
                    .insert(
                        map_key,
                        ImplementationMethod {
                            source_key,
                            declaration: method,
                        },
                    )
                    .is_some()
                {
                    return Err(format!(
                        "impl `{receiver}` declares method `{}` more than once",
                        method.name
                    ));
                }
            }
        }
    }
    Ok(methods)
}
