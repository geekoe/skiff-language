use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use sha2::Digest;

use crate::{
    validate_bytecode_statement_manifest_identity,
    validate_bytecode_statement_manifest_identity_lexical, PackageArtifact, PackageCallableId,
    PackageExecutableCoordinate, PackageLocalAbiSymbol, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

pub const PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-package-synthetic-callback-callable-identity-v1";
pub const PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_PREFIX: &str =
    "skiff-package-synthetic-callback-callable-v1:sha256";
pub const MAX_PACKAGE_SYNTHETIC_CALLBACK_OWNERS: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSyntheticCallbackOwner {
    pub owner: PackageExecutableCoordinate,
    pub site_ordinal: u32,
    pub package_callable_id: PackageCallableId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageBuildAuthorityValidationError {
    message: String,
}

impl PackageBuildAuthorityValidationError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PackageBuildAuthorityValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PackageBuildAuthorityValidationError {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyntheticCallbackCallableIdentityInput<'a> {
    schema: &'static str,
    package_id: &'a str,
    ordinary_owner_canonical_implementation_callable_id: &'a PackageCallableId,
    site_ordinal: u32,
}

/// Derives the only canonical callable id for one synthetic callback site.
/// The preimage deliberately excludes build, bytecode, function-key, path,
/// and provider identities.
pub fn derive_synthetic_callback_callable_id(
    package_id: &str,
    ordinary_owner_canonical_implementation_callable_id: &PackageCallableId,
    site_ordinal: u32,
) -> Result<PackageCallableId, PackageBuildAuthorityValidationError> {
    validate_lexical("packageId", package_id)?;
    validate_lexical(
        "ordinary owner canonical implementation callable id",
        ordinary_owner_canonical_implementation_callable_id.as_str(),
    )?;
    let bytes =
        skiff_canonical_json::canonical_json_bytes(&SyntheticCallbackCallableIdentityInput {
            schema: PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_SCHEMA_MARKER,
            package_id,
            ordinary_owner_canonical_implementation_callable_id,
            site_ordinal,
        })
        .map_err(|error| {
            PackageBuildAuthorityValidationError::new(format!(
                "failed to canonicalize synthetic callback callable identity: {error}"
            ))
        })?;
    Ok(PackageCallableId::new(format!(
        "{PACKAGE_SYNTHETIC_CALLBACK_CALLABLE_IDENTITY_PREFIX}:{}",
        hex::encode(sha2::Sha256::digest(bytes))
    )))
}

/// Validates PackageArtifact-local authority only. Exact cover against a
/// hydrated BytecodeArtifact and registry consistency across a deployment
/// closure remain paired attach/deployment/loader responsibilities.
pub fn validate_package_build_authority(
    artifact: &PackageArtifact,
) -> Result<(), PackageBuildAuthorityValidationError> {
    if artifact.schema_version != PACKAGE_ARTIFACT_SCHEMA_VERSION {
        return invalid(format!(
            "schemaVersion must be {PACKAGE_ARTIFACT_SCHEMA_VERSION}, got {}",
            artifact.schema_version
        ));
    }
    crate::validate_platform_error_projection_registry_ref_shape(
        &artifact.platform_error_projection_registry,
    )
    .map_err(|error| {
        PackageBuildAuthorityValidationError::new(format!(
            "invalid platformErrorProjectionRegistry: {error}"
        ))
    })?;
    validate_lexical("packageId", &artifact.package_id)?;
    validate_bytecode_statement_manifest_identity_lexical(
        &artifact.bytecode_statement_manifest_identity,
    )
    .map_err(|error| {
        PackageBuildAuthorityValidationError::new(format!(
            "invalid bytecodeStatementManifestIdentity: {error}"
        ))
    })?;
    if artifact.bytecode.is_none() {
        validate_bytecode_statement_manifest_identity(
            &artifact.package_id,
            &[],
            &artifact.bytecode_statement_manifest_identity,
        )
        .map_err(|error| {
            PackageBuildAuthorityValidationError::new(format!(
                "package without bytecode must declare the canonical empty statement manifest: {error}"
            ))
        })?;
    }
    let (public_callables, implementation_callables) = callable_surfaces(artifact)?;
    let ordinary_callables = public_callables
        .union(&implementation_callables)
        .cloned()
        .collect::<BTreeSet<_>>();
    let link_keys = artifact
        .callable_links
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if link_keys != ordinary_callables {
        return invalid(
            "callableLinks must exact-cover public and implementation callables and exclude synthetic callbacks",
        );
    }
    for (callable_id, link) in &artifact.callable_links {
        if callable_id != &link.callable_id || link.target.callable_abi_id != callable_id.as_str() {
            return invalid(format!(
                "callable link {callable_id} disagrees with its nested identity"
            ));
        }
    }

    let mut implementation_by_coordinate = BTreeMap::new();
    for callable_id in &implementation_callables {
        let link = artifact.callable_links.get(callable_id).ok_or_else(|| {
            PackageBuildAuthorityValidationError::new(format!(
                "implementation callable {callable_id} has no callable link"
            ))
        })?;
        let coordinate = PackageExecutableCoordinate {
            file_ir_identity: link.target.file_ref.file_ir_identity.clone(),
            module_path: link.target.file_ref.module_path.clone(),
            executable_index: link.target.executable_index,
        };
        validate_coordinate(&coordinate)?;
        if let Some(previous) = implementation_by_coordinate.insert(coordinate.clone(), callable_id)
        {
            return invalid(format!(
                "ordinary executable coordinate {coordinate:?} is owned by both {previous} and {callable_id}"
            ));
        }
    }

    if artifact.synthetic_callback_owners.len() as u64 > MAX_PACKAGE_SYNTHETIC_CALLBACK_OWNERS {
        return invalid(format!(
            "syntheticCallbackOwners exceeds {MAX_PACKAGE_SYNTHETIC_CALLBACK_OWNERS} rows"
        ));
    }
    let mut previous_site: Option<(&PackageExecutableCoordinate, u32)> = None;
    let mut synthetic_ids = BTreeSet::new();
    for row in &artifact.synthetic_callback_owners {
        validate_coordinate(&row.owner)?;
        let key = (&row.owner, row.site_ordinal);
        if previous_site.is_some_and(|previous| previous >= key) {
            return invalid(
                "syntheticCallbackOwners must be strictly ordered and unique by owner and siteOrdinal",
            );
        }
        previous_site = Some(key);
        let ordinary_id = implementation_by_coordinate
            .get(&row.owner)
            .ok_or_else(|| {
                PackageBuildAuthorityValidationError::new(format!(
                "synthetic callback owner {:?} is not a unique ordinary implementation coordinate",
                row.owner
            ))
            })?;
        let expected = derive_synthetic_callback_callable_id(
            &artifact.package_id,
            ordinary_id,
            row.site_ordinal,
        )?;
        if row.package_callable_id != expected {
            return invalid(format!(
                "synthetic callback at {:?}/{} declares {}, expected {expected}",
                row.owner, row.site_ordinal, row.package_callable_id
            ));
        }
        if ordinary_callables.contains(&row.package_callable_id)
            || !synthetic_ids.insert(row.package_callable_id.clone())
        {
            return invalid(format!(
                "synthetic callback id {} collides with an ordinary/public alias or another synthetic callback",
                row.package_callable_id
            ));
        }
    }

    let expected_semantic_facts = ordinary_callables
        .union(&synthetic_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual_semantic_facts = artifact
        .callable_semantic_facts
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual_semantic_facts != expected_semantic_facts {
        return invalid(
            "callableSemanticFacts must exact-cover ordinary and synthetic callable identities",
        );
    }

    super::schema_records::validate_bytecode_schema_records(
        &artifact.package_id,
        &artifact.bytecode_schema_records,
    )
}

pub(super) fn deserialize_synthetic_callback_owners<'de, D>(
    deserializer: D,
) -> Result<Vec<PackageSyntheticCallbackOwner>, D::Error>
where
    D: Deserializer<'de>,
{
    let rows = Vec::<PackageSyntheticCallbackOwner>::deserialize(deserializer)?;
    for adjacent in rows.windows(2) {
        let left = (&adjacent[0].owner, adjacent[0].site_ordinal);
        let right = (&adjacent[1].owner, adjacent[1].site_ordinal);
        if left >= right {
            return Err(D::Error::custom(
                "syntheticCallbackOwners must be strictly ordered and unique by owner and siteOrdinal",
            ));
        }
    }
    Ok(rows)
}

fn callable_surfaces(
    artifact: &PackageArtifact,
) -> Result<
    (BTreeSet<PackageCallableId>, BTreeSet<PackageCallableId>),
    PackageBuildAuthorityValidationError,
> {
    let public = collect_surface_callables(&artifact.package_local_abi.public_symbols, "public")?;
    let implementation = collect_surface_callables(
        &artifact.package_local_abi.implementation_symbols,
        "implementation",
    )?;
    if let Some(collision) = public.intersection(&implementation).next() {
        return invalid(format!(
            "callable identity {collision} is owned by public and implementation surfaces"
        ));
    }
    Ok((public, implementation))
}

fn collect_surface_callables(
    symbols: &BTreeMap<String, PackageLocalAbiSymbol>,
    surface: &str,
) -> Result<BTreeSet<PackageCallableId>, PackageBuildAuthorityValidationError> {
    let mut callables = BTreeSet::new();
    for (path, symbol) in symbols {
        let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
            continue;
        };
        validate_lexical(&format!("{surface} callable path"), path)?;
        validate_lexical(&format!("{surface} callable id"), callable_id.as_str())?;
        if !callables.insert(callable_id.clone()) {
            return invalid(format!("{surface} repeats callable id {callable_id}"));
        }
    }
    Ok(callables)
}

fn validate_coordinate(
    coordinate: &PackageExecutableCoordinate,
) -> Result<(), PackageBuildAuthorityValidationError> {
    validate_lexical("owner.fileIrIdentity", &coordinate.file_ir_identity)?;
    validate_lexical("owner.modulePath", &coordinate.module_path)
}

fn validate_lexical(label: &str, value: &str) -> Result<(), PackageBuildAuthorityValidationError> {
    if value.is_empty()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return invalid(format!(
            "{label} must be non-empty and contain no whitespace or control characters"
        ));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, PackageBuildAuthorityValidationError> {
    Err(PackageBuildAuthorityValidationError::new(message))
}
