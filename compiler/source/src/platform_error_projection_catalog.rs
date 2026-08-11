use skiff_artifact_model::{NamedUnionBranchIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr};
use skiff_compiler_input::{
    CompilerPlatformSources, CompilerPlatformSourcesError, PlatformErrorProjectionCatalog,
    PlatformErrorProjectionCatalogEntry,
};
use thiserror::Error;

use crate::{
    prelude_registry::{
        initialize_prelude_registry, PreludeRegistry, PreludeRegistryInitializationError,
    },
    runtime_type_projection::lower_prelude_type_decl,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPlatformErrorProjectionCatalog {
    entries: Vec<ResolvedPlatformErrorProjectionEntry>,
}

impl ResolvedPlatformErrorProjectionCatalog {
    pub fn entries(&self) -> &[ResolvedPlatformErrorProjectionEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPlatformErrorProjectionEntry {
    projection_key: String,
    nominal_identity: String,
    canonical_public_type_ir: TypeDeclIr,
    producer_family: String,
    semantic_adapter_owner: String,
    public_message_policy: String,
    envelope_kind: String,
    fallback_policy: String,
}

impl ResolvedPlatformErrorProjectionEntry {
    pub fn projection_key(&self) -> &str {
        &self.projection_key
    }

    pub fn nominal_identity(&self) -> &str {
        &self.nominal_identity
    }

    pub fn canonical_public_type_ir(&self) -> &TypeDeclIr {
        &self.canonical_public_type_ir
    }

    pub fn producer_family(&self) -> &str {
        &self.producer_family
    }

    pub fn semantic_adapter_owner(&self) -> &str {
        &self.semantic_adapter_owner
    }

    pub fn public_message_policy(&self) -> &str {
        &self.public_message_policy
    }

    pub fn envelope_kind(&self) -> &str {
        &self.envelope_kind
    }

    pub fn fallback_policy(&self) -> &str {
        &self.fallback_policy
    }
}

#[derive(Debug, Error)]
pub enum PlatformErrorProjectionCatalogError {
    #[error(transparent)]
    PlatformSources(#[from] CompilerPlatformSourcesError),
    #[error(transparent)]
    PreludeRegistry(#[from] PreludeRegistryInitializationError),
    #[error("platform error projection key {projection_key} is not a known canonical symbol")]
    UnknownProjectionSymbol { projection_key: String },
    #[error(
        "platform error projection key {projection_key} resolves to canonical symbol {resolved_symbol}, not itself"
    )]
    NonCanonicalProjectionSymbol {
        projection_key: String,
        resolved_symbol: String,
    },
    #[error(
        "platform error projection key {projection_key} names a type alias, not a type declaration"
    )]
    AliasDeclaration { projection_key: String },
    #[error(
        "platform error projection key {projection_key} does not name an exact source type declaration"
    )]
    NotTypeDeclaration { projection_key: String },
    #[error("platform error projection type {projection_key} is not an exact public declaration")]
    NonPublicTypeDeclaration { projection_key: String },
    #[error("platform error projection type {projection_key} must not declare type parameters")]
    GenericTypeDeclaration { projection_key: String },
    #[error("failed to lower platform error projection type {projection_key}: {message}")]
    TypeLowering {
        projection_key: String,
        message: String,
    },
    #[error("platform error projection type {projection_key} is not a closed payload: {message}")]
    OpenPayload {
        projection_key: String,
        message: String,
    },
}

pub fn resolve_platform_error_projection_catalog(
    platform_sources: &CompilerPlatformSources,
) -> Result<ResolvedPlatformErrorProjectionCatalog, PlatformErrorProjectionCatalogError> {
    let catalog = platform_sources.read_platform_error_projection_catalog()?;
    let registry = initialize_prelude_registry(platform_sources)?;
    resolve_catalog_against_registry(&catalog, registry)
}

fn resolve_catalog_against_registry(
    catalog: &PlatformErrorProjectionCatalog,
    registry: &PreludeRegistry,
) -> Result<ResolvedPlatformErrorProjectionCatalog, PlatformErrorProjectionCatalogError> {
    let entries = catalog
        .entries()
        .iter()
        .map(|entry| resolve_entry(entry, registry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ResolvedPlatformErrorProjectionCatalog { entries })
}

fn resolve_entry(
    entry: &PlatformErrorProjectionCatalogEntry,
    registry: &PreludeRegistry,
) -> Result<ResolvedPlatformErrorProjectionEntry, PlatformErrorProjectionCatalogError> {
    let projection_key = entry.projection_key();
    let Some(resolved_symbol) = registry.known_type_symbol(projection_key) else {
        return Err(
            PlatformErrorProjectionCatalogError::UnknownProjectionSymbol {
                projection_key: projection_key.to_string(),
            },
        );
    };
    if resolved_symbol != projection_key {
        return Err(
            PlatformErrorProjectionCatalogError::NonCanonicalProjectionSymbol {
                projection_key: projection_key.to_string(),
                resolved_symbol,
            },
        );
    }
    if registry.exact_type_alias(projection_key).is_some() {
        return Err(PlatformErrorProjectionCatalogError::AliasDeclaration {
            projection_key: projection_key.to_string(),
        });
    }
    let Some(declaration) = registry.exact_type_decl(projection_key) else {
        return Err(PlatformErrorProjectionCatalogError::NotTypeDeclaration {
            projection_key: projection_key.to_string(),
        });
    };
    if !registry.is_public_type_declaration(projection_key) {
        return Err(
            PlatformErrorProjectionCatalogError::NonPublicTypeDeclaration {
                projection_key: projection_key.to_string(),
            },
        );
    }
    if !declaration.type_params.is_empty() {
        return Err(
            PlatformErrorProjectionCatalogError::GenericTypeDeclaration {
                projection_key: projection_key.to_string(),
            },
        );
    }

    let canonical_public_type_ir = lower_prelude_type_decl(declaration).map_err(|message| {
        PlatformErrorProjectionCatalogError::TypeLowering {
            projection_key: projection_key.to_string(),
            message,
        }
    })?;
    validate_closed_payload(&canonical_public_type_ir).map_err(|message| {
        PlatformErrorProjectionCatalogError::OpenPayload {
            projection_key: projection_key.to_string(),
            message,
        }
    })?;

    Ok(ResolvedPlatformErrorProjectionEntry {
        projection_key: projection_key.to_string(),
        nominal_identity: projection_key.to_string(),
        canonical_public_type_ir,
        producer_family: entry.producer_family().to_string(),
        semantic_adapter_owner: entry.semantic_adapter_owner().to_string(),
        public_message_policy: entry.public_message_policy().to_string(),
        envelope_kind: entry.envelope_kind().to_string(),
        fallback_policy: entry.fallback_policy().to_string(),
    })
}

fn validate_closed_payload(declaration: &TypeDeclIr) -> Result<(), String> {
    if !declaration.type_params.is_empty() {
        return Err("canonical type IR contains type parameters".to_string());
    }
    if declaration.source_span.is_some() {
        return Err("canonical type IR contains a source span".to_string());
    }
    if !declaration.implements.is_empty() {
        return Err("canonical type IR contains nominal interface references".to_string());
    }
    match &declaration.descriptor {
        TypeDescriptorIr::Record { fields } => {
            for (field, ty) in fields {
                validate_closed_field_type(ty)
                    .map_err(|message| format!("field {field}: {message}"))?;
            }
            Ok(())
        }
        TypeDescriptorIr::Union { branches } => {
            for (index, branch) in branches.iter().enumerate() {
                validate_closed_union_branch(branch)
                    .map_err(|message| format!("union branch {index}: {message}"))?;
            }
            Ok(())
        }
        TypeDescriptorIr::Representation { .. } => {
            Err("representation declarations are forbidden".to_string())
        }
        TypeDescriptorIr::Alias { .. } => Err("type aliases are forbidden".to_string()),
        TypeDescriptorIr::Interface => Err("interfaces are forbidden".to_string()),
    }
}

fn validate_closed_union_branch(branch: &NamedUnionBranchIr) -> Result<(), String> {
    match branch {
        NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type,
            discriminator_field,
            discriminator_value,
        } => {
            let TypeRefIr::Record { fields } = payload_type else {
                return Err("synthetic discriminator payload must be a record".to_string());
            };
            let Some(TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::String { value },
            }) = fields.get(discriminator_field)
            else {
                return Err("synthetic discriminator field must be a string literal".to_string());
            };
            if value != discriminator_value {
                return Err("synthetic discriminator value does not match its field".to_string());
            }
            validate_closed_field_type(payload_type)
        }
        NamedUnionBranchIr::Literal { .. } => Ok(()),
        NamedUnionBranchIr::ConcreteNominal { .. } => {
            Err("nominal union branches are forbidden".to_string())
        }
    }
}

fn validate_closed_field_type(ty: &TypeRefIr) -> Result<(), String> {
    match ty {
        TypeRefIr::Builtin { name, args }
            if args.is_empty()
                && matches!(
                    name.as_str(),
                    "string" | "integer" | "number" | "bool" | "boolean" | "Json"
                ) =>
        {
            Ok(())
        }
        TypeRefIr::Builtin { name, .. } => Err(format!("builtin type {name} is forbidden")),
        TypeRefIr::Record { fields } => {
            for (field, field_type) in fields {
                validate_closed_field_type(field_type)
                    .map_err(|message| format!("record field {field}: {message}"))?;
            }
            Ok(())
        }
        TypeRefIr::Union { items } => {
            for (index, item) in items.iter().enumerate() {
                validate_closed_field_type(item)
                    .map_err(|message| format!("union item {index}: {message}"))?;
            }
            Ok(())
        }
        TypeRefIr::Nullable { inner } => validate_closed_field_type(inner),
        TypeRefIr::Literal { .. } => Ok(()),
        TypeRefIr::Function { .. } => Err("function types are forbidden".to_string()),
        TypeRefIr::TypeParam { .. } => Err("type parameters are forbidden".to_string()),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::AppliedNominal { .. } => Err("nominal references are forbidden".to_string()),
        TypeRefIr::DbObjectSymbol { .. } => {
            Err("database object handles are forbidden".to_string())
        }
        TypeRefIr::AnyInterface { .. } => Err("capability interfaces are forbidden".to_string()),
    }
}

#[cfg(test)]
#[path = "platform_error_projection_catalog/tests.rs"]
mod tests;
