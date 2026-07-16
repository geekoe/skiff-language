use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::ServiceUnit;

use crate::framing::{identity, sha256_hex};
use crate::{ArtifactIdentityError, Result, SERVICE_UNIT_IDENTITY_PREFIX};
use skiff_canonical_json::canonical_json_value;

pub fn service_unit_hash(unit: &ServiceUnit) -> Result<String> {
    Ok(sha256_hex(&service_unit_identity_bytes(unit)?))
}

pub fn service_unit_identity(unit: &ServiceUnit) -> Result<String> {
    Ok(identity(
        SERVICE_UNIT_IDENTITY_PREFIX,
        &service_unit_hash(unit)?,
    ))
}

pub fn service_unit_identity_value(unit: &ServiceUnit) -> Result<Value> {
    let value = serde_json::to_value(ServiceUnitStorageIdentityPayload {
        identity_schema: "skiff-service-unit-identity-v1",
        unit,
    })
    .map_err(ArtifactIdentityError::SerializeServiceUnitStorageIdentity)?;
    Ok(canonical_json_value(&value))
}

pub fn service_unit_identity_bytes(unit: &ServiceUnit) -> Result<Vec<u8>> {
    let value = service_unit_identity_value(unit)?;
    serde_json::to_vec(&value).map_err(ArtifactIdentityError::SerializeServiceUnitStorageIdentity)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceUnitStorageIdentityPayload<'a> {
    identity_schema: &'static str,
    unit: &'a ServiceUnit,
}
