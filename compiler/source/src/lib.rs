pub mod alias_resolution;
pub mod api;
pub(crate) mod api_seed;
mod api_yml;
mod compile_model;
mod config_metadata;
pub(crate) mod config_requirements;
pub(crate) mod config_usage;
mod dependency_operation_facts;
pub mod entity;
pub(crate) mod expression_model;
pub(crate) mod expression_type_model;
mod linked_facts;
pub(crate) mod linked_publication;
mod name_resolution_model;
mod package_dependency_facts;
mod package_export_resolver;
pub mod package_rules;
pub mod parsed_sources;
pub mod prelude_registry;
pub mod provider_rules;
mod remote_public_instance;
pub mod reserved_names;
pub mod root_projection_validation;
pub mod root_refs;
pub(crate) mod runtime_type_projection;
pub mod semantic;
mod semantics;
pub mod service_ingress;
pub mod service_package_facts;
pub(crate) mod service_rules;
pub(crate) mod service_storage_rules;
pub(crate) mod shared;
mod source_file_facts;
pub mod source_graph;
pub mod source_identity;
pub mod source_name_resolution;
pub(crate) mod source_rules;
mod test_rules;
pub(crate) mod type_resolution_model;
mod type_symbol_index;

use std::collections::BTreeMap;

use crate::shared::{
    ast::{SourceFile, TypeRef},
    publication_error::PublicationError,
    type_syntax::generic_parts,
};
pub use compiler_input_model::{
    PackageDependency, PublicationApiEntry, PublicationApiSpec, PublicationCompilePolicy,
    ResolvedServiceDependencies,
};

pub use api::{PublicationApi, SourceSymbolKey};
pub use api_seed::PublicationApiSeed;
pub use compile_model::{
    ExportBindingModel, ExportCallableBinding, ExportPublicInstanceBinding,
    ExportPublicInstanceInterfaceBinding, ExportSchemaBinding, ExportSymbolBinding,
    PublicationApiModel, PublicationSourceSet, ResolutionModels, ResolvedDependencies,
    SourceCompileModel, SourceCompileModelInput, SourceIndexes,
};
pub use config_metadata::{
    source_config_and_effect_metadata_batches_from_parsed_sources,
    source_config_and_effect_metadata_from_parsed_sources, SourceConfigAndEffectMetadata,
    SourceConfigAndEffectMetadataBatchInput, SourceConfigAndEffectMetadataInput,
    SourceConfigMetadata, SourceEffectMetadata,
};
pub use config_requirements::{
    ConfigRequirement, ConfigRequirementAccess, ConfigRequirementDependencyStep,
    ConfigRequirementScope, ConfigRequirementSet, DependencyPackageConfigFacts,
};
pub use config_usage::ConfigSourceSpan;
pub use dependency_operation_facts::DependencyPackageOperationFacts;
pub use expression_model::{
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceFact, ExpressionSourceMap,
};
pub use expression_type_model::{
    ConstructorFieldTypeMismatch, ConstructorFieldValueSource, ConstructorProvidedField,
    ConstructorValidation, DuplicateConstructorField, ExpressionTypeFact, ExpressionTypeModel,
    ExpressionTypeModelBuildError, MaterializedConstructorField, MissingConstructorField,
    RepresentationConstructorValidation, UnknownConstructorField,
};
pub use linked_facts::{SourceCompileLinkedFacts, SourceCompileLinkedFactsInput};
pub use linked_publication::CompileParsedPublicationSourcesInput;
pub use name_resolution_model::{validate_source_name_resolution_from_model, NameResolutionModel};
pub use package_dependency_facts::{SourceCompilePackageDependencyFact, SourceCompilePackageFacts};
pub use remote_public_instance::{
    RemotePublicInstanceDirectOperation, RemotePublicInstanceOperationProjection,
    RemotePublicInstanceOperationResolver, RemotePublicInstanceOperationSlot,
};
pub use semantics::PublicationCompilePlan;
pub use service_ingress::{
    ServiceHttpIngressInput, ServiceHttpRouteIngress, ServiceHttpRouteIngressInput,
    ServiceIngressHandler, ServiceIngressInput, ServiceIngressModel, ServiceWebSocketIngress,
    ServiceWebSocketIngressInput,
};
pub use shared::publication_error::PublicationError as SourceCompileError;
pub use source_file_facts::{
    publication_db_metadata_index, type_indices, type_text_with_args, LocalDbObjectIndex,
    PackageInterfaceMethodIndex, PublicationDbMetadata, PublicationDbMetadataIndex,
};
pub use type_resolution_model::{
    AnyInterfaceMethodResolution, ConstructorTargetResolution,
    LocalAnyInterfaceConformanceResolution, PackageCallableResolution,
    RepresentationConstructorResolution, ResolvedTypeRef, TypeResolutionContext,
    TypeResolutionModel, TypeResolutionPackageCallableFact, TypeResolutionPackageDependencyFact,
    TypeResolutionPackageFacts, TypeResolutionPackageSchemaTypeFact,
};
pub use type_symbol_index::{publication_type_symbols, PublicationTypeSymbolIndex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationKind {
    Package,
    Service,
}

pub fn build_from_parsed_sources(
    input: CompileParsedPublicationSourcesInput<'_, '_>,
) -> Result<SourceCompileModel, PublicationError> {
    let linked = linked_publication::LinkedPublication::from_parsed_sources(input);
    build_from_linked(linked)
}

fn build_from_linked(
    linked: linked_publication::LinkedPublication<'_, '_>,
) -> Result<SourceCompileModel, PublicationError> {
    let root_ref_policy = match linked.policy {
        PublicationCompilePolicy::Package { .. } => {
            root_refs::RootRefValidationPolicy::parsed_publication_sources()
        }
        PublicationCompilePolicy::Service { .. } => {
            root_refs::RootRefValidationPolicy::service_sources()
        }
    };
    root_refs::validate_source_root_refs(
        linked.diagnostic_root,
        &linked.production_sources,
        root_ref_policy,
    )?;
    let parsed_sources = linked.parsed_sources;
    if matches!(linked.policy, PublicationCompilePolicy::Service { .. }) {
        service_storage_rules::validate_service_storage_sources(&parsed_sources)?;
    }
    let dependency_package_config_facts = linked.package_facts.map(dependency_package_config_facts);
    let type_resolution_package_facts = linked.package_facts.map(type_resolution_package_facts);
    let package_db_metadata_index =
        package_db_metadata_index(linked.package_facts, linked.package_dependencies);
    let dependency_package_operation_facts =
        DependencyPackageOperationFacts::from_package_facts(linked.package_facts)?;
    let source_identity = source_identity::source_identity(&parsed_sources);
    let publication_id_for_anchors = match &linked.policy {
        PublicationCompilePolicy::Package { package_id } => *package_id,
        PublicationCompilePolicy::Service { service_id } => *service_id,
    };
    let declaration_anchors = source_identity::PublicationDeclarationAnchors::build(
        &parsed_sources,
        publication_id_for_anchors,
    );
    let publication_kind = publication_kind_for_policy(linked.policy);
    let entity_model = entity::PublicationEntityModel::from_declaration_anchors(
        entity_publication_kind(publication_kind),
        declaration_anchors.anchors(),
    );
    let service_alias_set = linked.service_dependencies.aliases();
    let name_resolution = NameResolutionModel::build_with(
        &parsed_sources,
        linked.package_aliases,
        &service_alias_set,
        Some(entity_model.top_level()),
    );
    validate_source_name_resolution_from_model(
        linked.diagnostic_root,
        &parsed_sources,
        &name_resolution,
    )?;
    if matches!(linked.policy, PublicationCompilePolicy::Service { .. }) {
        service_rules::validate_service_publication_sources_with_name_resolution(
            linked.diagnostic_root,
            &parsed_sources,
            &name_resolution,
        )?;
    }
    let linked_facts = SourceCompileLinkedFacts::build(SourceCompileLinkedFactsInput {
        diagnostic_root: linked.diagnostic_root,
        parsed_sources: &parsed_sources,
        production_sources: &linked.production_sources,
        package_dependencies: linked.package_dependencies,
        dependency_package_config_facts: dependency_package_config_facts.as_deref(),
        policy: linked.policy,
        publication_api: linked.publication_api,
    })?;
    let config_usage_seed = config_usage::collect_config_usage_seed_from_parsed_sources(
        linked.diagnostic_root,
        &parsed_sources,
    )?;
    SourceCompileModel::build(SourceCompileModelInput {
        parsed_sources,
        diagnostic_root: linked.diagnostic_root,
        package_aliases: linked.package_aliases,
        package_dependencies: linked.package_dependencies,
        package_db_metadata_index,
        type_resolution_package_facts: type_resolution_package_facts.as_deref(),
        dependency_package_operation_facts,
        service_dependencies: linked.service_dependencies,
        service_ingress: linked.service_ingress,
        entity_model,
        name_resolution,
        policy: linked.policy,
        publication_api_seed: linked_facts.publication_api_seed,
        source_identity,
        declaration_anchors,
        config_usage_seed,
        dependency_config_requirements: linked_facts.dependency_config_requirements,
    })
}

fn publication_kind_for_policy(policy: PublicationCompilePolicy<'_>) -> PublicationKind {
    match policy {
        PublicationCompilePolicy::Package { .. } => PublicationKind::Package,
        PublicationCompilePolicy::Service { .. } => PublicationKind::Service,
    }
}

fn entity_publication_kind(kind: PublicationKind) -> entity::PublicationKind {
    match kind {
        PublicationKind::Package => entity::PublicationKind::Package,
        PublicationKind::Service => entity::PublicationKind::Service,
    }
}

fn type_resolution_package_facts<'facts>(
    package_facts: &'facts [SourceCompilePackageFacts<'_>],
) -> Vec<TypeResolutionPackageFacts<'facts>> {
    package_facts
        .iter()
        .map(type_resolution_package_fact)
        .collect()
}

fn type_resolution_package_fact<'facts>(
    package: &'facts SourceCompilePackageFacts<'_>,
) -> TypeResolutionPackageFacts<'facts> {
    let compiled = package.compile_model();
    let parsed_sources = compiled.sources().parsed_sources();
    let file_ir_units = package.file_ir_units();
    TypeResolutionPackageFacts {
        package_id: package.id(),
        dependencies: package
            .dependencies()
            .iter()
            .map(|dependency| TypeResolutionPackageDependencyFact {
                alias: dependency.effective_alias(),
                package_id: dependency.id.as_str(),
            })
            .collect(),
        schema_types: compiled
            .export_bindings()
            .public_schema_types()
            .values()
            .filter_map(|binding| {
                let source_ast = parsed_source_ast(parsed_sources, &binding.source_module)?;
                let file_ir_unit = file_ir_units
                    .iter()
                    .find(|unit| unit.module_path == binding.source_module);
                Some(TypeResolutionPackageSchemaTypeFact {
                    public_path: binding.public_path.as_str(),
                    source_module: binding.source_module.as_str(),
                    source_symbol: binding.source_symbol.as_str(),
                    kind: binding.kind,
                    source_ast,
                    file_ir_unit,
                })
            })
            .collect(),
        callables: compiled
            .export_bindings()
            .public_callables()
            .values()
            .filter_map(|binding| {
                let source_ast = parsed_source_ast(parsed_sources, &binding.source_module)?;
                Some(TypeResolutionPackageCallableFact {
                    public_path: binding.public_path.as_str(),
                    source_module: binding.source_module.as_str(),
                    source_symbol: binding.source_symbol.as_str(),
                    source_ast,
                })
            })
            .collect(),
    }
}

fn parsed_source_ast<'a>(
    parsed_sources: &'a [parsed_sources::ParsedCompilerSource],
    module_path: &str,
) -> Option<&'a SourceFile> {
    parsed_sources
        .iter()
        .find(|source| source.source().module_path == module_path)
        .map(parsed_sources::ParsedCompilerSource::ast)
}

fn package_db_metadata_index(
    package_facts: Option<&[SourceCompilePackageFacts<'_>]>,
    package_dependencies: &[PackageDependency],
) -> Option<PublicationDbMetadataIndex> {
    let package_facts = package_facts?;
    let mut index = PublicationDbMetadataIndex::default();
    for package in package_facts {
        let aliases = package_dependency_aliases(package, package_dependencies);
        if aliases.is_empty() {
            continue;
        }
        let public_schema_types = package_public_schema_types_by_source(package);
        let type_name_mappings = package_type_name_mappings(package, &aliases);
        for (_, metadata) in package
            .compile_model()
            .indexes()
            .publication_db_metadata_index()
            .entries()
        {
            let Some(public_path) = public_schema_types
                .get(&(metadata.module_path.clone(), metadata.type_name.clone()))
            else {
                continue;
            };
            let metadata = package_service_visible_db_metadata(metadata, &type_name_mappings);
            for alias in &aliases {
                let alias_path = alias_public_path(alias, public_path);
                let (module_path, source_name) = split_public_path(&alias_path);
                index.insert_alias(&module_path, &source_name, metadata.clone());
            }
        }
    }
    Some(index)
}

fn package_dependency_aliases<'a>(
    package: &SourceCompilePackageFacts<'_>,
    package_dependencies: &'a [PackageDependency],
) -> Vec<&'a str> {
    package_dependencies
        .iter()
        .filter(|dependency| {
            dependency.id == package.id() && dependency.version == package.version()
        })
        .map(PackageDependency::effective_alias)
        .collect()
}

fn package_public_schema_types_by_source(
    package: &SourceCompilePackageFacts<'_>,
) -> BTreeMap<(String, String), String> {
    package
        .compile_model()
        .export_bindings()
        .public_schema_types()
        .values()
        .map(|binding| {
            (
                (binding.source_module.clone(), binding.source_symbol.clone()),
                binding.public_path.clone(),
            )
        })
        .collect()
}

fn package_type_name_mappings(
    package: &SourceCompilePackageFacts<'_>,
    aliases: &[&str],
) -> BTreeMap<String, String> {
    let mut mappings = BTreeMap::new();
    for binding in package
        .compile_model()
        .export_bindings()
        .public_schema_types()
        .values()
    {
        for alias in aliases {
            let visible_name = alias_public_path(alias, &binding.public_path);
            insert_type_name_mapping(&mut mappings, &binding.source_symbol, &visible_name);
            insert_type_name_mapping(
                &mut mappings,
                &format!("{}.{}", binding.source_module, binding.source_symbol),
                &visible_name,
            );
            insert_type_name_mapping(&mut mappings, &binding.public_path, &visible_name);
        }
    }
    mappings
}

fn insert_type_name_mapping(mappings: &mut BTreeMap<String, String>, from: &str, to: &str) {
    if from.is_empty() {
        return;
    }
    mappings.insert(from.to_string(), to.to_string());
    mappings.insert(format!("root.{from}"), to.to_string());
}

fn package_service_visible_db_metadata(
    metadata: &PublicationDbMetadata,
    type_name_mappings: &BTreeMap<String, String>,
) -> PublicationDbMetadata {
    let mut metadata = metadata.clone();
    metadata.key.ty = package_service_visible_type_ref(&metadata.key.ty, type_name_mappings);
    metadata.field_types = metadata
        .field_types
        .into_iter()
        .map(|(field, ty)| {
            (
                field,
                package_service_visible_type_ref(&ty, type_name_mappings),
            )
        })
        .collect();
    metadata.field_type_texts = metadata
        .field_type_texts
        .into_iter()
        .map(|(field, ty)| {
            (
                field,
                package_service_visible_type_text(&ty, type_name_mappings),
            )
        })
        .collect();
    metadata
}

fn package_service_visible_type_ref(
    ty: &TypeRef,
    type_name_mappings: &BTreeMap<String, String>,
) -> TypeRef {
    TypeRef {
        name: package_service_visible_type_text(&ty.name, type_name_mappings),
    }
}

fn package_service_visible_type_text(
    ty: &str,
    type_name_mappings: &BTreeMap<String, String>,
) -> String {
    let ty = ty.trim();
    if let Some(mapped) = type_name_mappings.get(ty) {
        return mapped.clone();
    }
    if let Some(parts) = generic_parts(ty) {
        let args = parts
            .args
            .iter()
            .map(|arg| package_service_visible_type_text(arg, type_name_mappings))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{}<{args}>", parts.root);
    }
    ty.to_string()
}

fn alias_public_path(alias: &str, public_path: &str) -> String {
    if public_path == alias || public_path.starts_with(&format!("{alias}.")) {
        public_path.to_string()
    } else {
        format!("{alias}.{public_path}")
    }
}

fn split_public_path(path: &str) -> (String, String) {
    path.rsplit_once('.')
        .map(|(module_path, source_name)| (module_path.to_string(), source_name.to_string()))
        .unwrap_or_else(|| (String::new(), path.to_string()))
}

fn dependency_package_config_facts<'facts>(
    package_facts: &'facts [SourceCompilePackageFacts<'_>],
) -> Vec<config_requirements::DependencyPackageConfigFacts<'facts>> {
    package_facts
        .iter()
        .map(
            |package| config_requirements::DependencyPackageConfigFacts {
                id: package.id(),
                version: package.version(),
                dependencies: package
                    .dependencies()
                    .iter()
                    .map(
                        config_requirements::ConfigRequirementDependencyStep::from_package_dependency_fact,
                    )
                    .collect(),
                own_config_requirements: package.compile_model().own_config_requirements(),
            },
        )
        .collect()
}
