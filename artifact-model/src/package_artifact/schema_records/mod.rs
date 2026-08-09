use std::collections::BTreeMap;

use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserializer, Serialize};
use sha2::Digest;

use crate::{PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord};

use super::authority::PackageBuildAuthorityValidationError;

mod canonical;
mod graph;

pub const PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-package-schema-type-identity-v2";
pub const PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX: &str = "skiff-package-schema-type-v2:sha256";
pub const MAX_BYTECODE_SCHEMA_RECORDS: u64 = 1_000_000;
pub const MAX_BYTECODE_SCHEMA_TYPE_NODES: u64 = 1_000_000;
pub const MAX_BYTECODE_SCHEMA_CANONICAL_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_BYTECODE_SCHEMA_DEPTH: u32 = 64;
pub const MAX_BYTECODE_SCHEMA_STRING_BYTES: u64 = 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageSchemaTypeIdentityInput<'a> {
    schema: &'static str,
    package_id: &'a str,
    stable_schema_key: &'a str,
    canonical_descriptor: &'a PackageSchemaCanonicalDescriptor,
}

/// Derives the existing v2 PackageSchema content identity from one already
/// canonical descriptor. The bytecode closure reuses this authority; it does
/// not define a parallel lifecycle or shape identity.
pub fn derive_package_schema_type_id(
    package_id: &str,
    stable_schema_key: &str,
    canonical_descriptor: &PackageSchemaCanonicalDescriptor,
) -> Result<PackageSchemaTypeId, PackageBuildAuthorityValidationError> {
    graph::validate_identity_inputs(package_id, stable_schema_key, canonical_descriptor)?;
    graph::validate_single_descriptor(canonical_descriptor)?;
    canonical::validate_descriptor(canonical_descriptor)?;
    derive_canonical_id(package_id, stable_schema_key, canonical_descriptor)
}

/// Validates the self-contained descriptor graph carried by PackageArtifact.
/// Reachability from exact bytecode types is intentionally not checked here;
/// paired attach/loader validation owns that exact-cover proof.
pub fn validate_bytecode_schema_records(
    package_id: &str,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<(), PackageBuildAuthorityValidationError> {
    if records.len() as u64 > MAX_BYTECODE_SCHEMA_RECORDS {
        return invalid(format!(
            "bytecodeSchemaRecords exceeds {MAX_BYTECODE_SCHEMA_RECORDS} entries"
        ));
    }
    let canonical_bytes = skiff_canonical_json::canonical_json_bytes(records).map_err(|error| {
        PackageBuildAuthorityValidationError::new(format!(
            "failed to canonicalize bytecodeSchemaRecords: {error}"
        ))
    })?;
    if canonical_bytes.len() as u64 > MAX_BYTECODE_SCHEMA_CANONICAL_BYTES {
        return invalid(format!(
            "bytecodeSchemaRecords exceeds {MAX_BYTECODE_SCHEMA_CANONICAL_BYTES} canonical bytes"
        ));
    }

    graph::validate_graph(package_id, records)?;
    for (type_id, record) in records {
        if type_id != &record.package_schema_type_id {
            return invalid(format!(
                "bytecode schema record key {type_id} does not match nested identity {}",
                record.package_schema_type_id
            ));
        }
        canonical::validate_descriptor(&record.canonical_descriptor)?;
        let expected = derive_canonical_id(
            &record.package_id,
            &record.stable_schema_key,
            &record.canonical_descriptor,
        )?;
        if type_id != &expected {
            return invalid(format!(
                "bytecode schema record {} declares {type_id}, expected {expected}",
                record.stable_schema_key
            ));
        }
    }
    Ok(())
}

pub(super) fn deserialize_bytecode_schema_records<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>, D::Error>
where
    D: Deserializer<'de>,
{
    struct CanonicalRecordsVisitor;

    impl<'de> Visitor<'de> for CanonicalRecordsVisitor {
        type Value = BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a strictly ordered map of exact PackageSchema records")
        }

        fn visit_map<A>(
            self,
            mut access: A,
        ) -> Result<BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut records = BTreeMap::<PackageSchemaTypeId, PackageSchemaTypeRecord>::new();
            let mut previous: Option<PackageSchemaTypeId> = None;
            while let Some((type_id, record)) = access.next_entry()? {
                if previous
                    .as_ref()
                    .is_some_and(|previous| previous >= &type_id)
                {
                    return Err(A::Error::custom(
                        "bytecodeSchemaRecords keys must be strictly ordered and unique",
                    ));
                }
                previous = Some(type_id.clone());
                records.insert(type_id, record);
                if records.len() as u64 > MAX_BYTECODE_SCHEMA_RECORDS {
                    return Err(A::Error::custom(format!(
                        "bytecodeSchemaRecords exceeds {MAX_BYTECODE_SCHEMA_RECORDS} entries"
                    )));
                }
            }
            if let Some(owner) = records
                .values()
                .next()
                .map(|record| record.package_id.clone())
            {
                validate_bytecode_schema_records(&owner, &records).map_err(A::Error::custom)?;
            }
            Ok(records)
        }
    }

    deserializer.deserialize_map(CanonicalRecordsVisitor)
}

fn derive_canonical_id(
    package_id: &str,
    stable_schema_key: &str,
    canonical_descriptor: &PackageSchemaCanonicalDescriptor,
) -> Result<PackageSchemaTypeId, PackageBuildAuthorityValidationError> {
    let bytes = skiff_canonical_json::canonical_json_bytes(&PackageSchemaTypeIdentityInput {
        schema: PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER,
        package_id,
        stable_schema_key,
        canonical_descriptor,
    })
    .map_err(|error| {
        PackageBuildAuthorityValidationError::new(format!(
            "failed to canonicalize PackageSchema identity: {error}"
        ))
    })?;
    if bytes.len() as u64 > MAX_BYTECODE_SCHEMA_CANONICAL_BYTES {
        return invalid(format!(
            "PackageSchema identity preimage exceeds {MAX_BYTECODE_SCHEMA_CANONICAL_BYTES} canonical bytes"
        ));
    }
    Ok(PackageSchemaTypeId::new(format!(
        "{PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX}:{}",
        hex::encode(sha2::Sha256::digest(bytes))
    )))
}

fn invalid<T>(message: impl Into<String>) -> Result<T, PackageBuildAuthorityValidationError> {
    Err(PackageBuildAuthorityValidationError::new(message))
}
