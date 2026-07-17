use std::{fs, path::Path};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skiff_artifact_identity::{
    package_build_identity, package_local_abi_identity, package_unit_content_hash,
    publication_storage_segment, service_assembly_identity, service_unit_hash,
    service_unit_identity, PackageUnitArtifactRef, ServiceAssemblyArtifactRef,
    ServiceUnitArtifactRef,
};
use skiff_artifact_model::{PackageUnit, ServiceUnit};

pub(super) fn write_service_unit_ref(
    root: &Path,
    service_id: &str,
    source_path: &str,
) -> ServiceUnitArtifactRef {
    let value = read_json(root, source_path);
    let parsed = serde_json::from_value::<ServiceUnit>(value.clone());
    let (unit_hash, unit_identity) = match parsed {
        Ok(unit) => (
            service_unit_hash(&unit).expect("test service unit hash should compute"),
            service_unit_identity(&unit).expect("test service unit identity should compute"),
        ),
        Err(_) => {
            let hash = sha256_hex(
                &serde_json::to_vec(&value).expect("test service unit should serialize"),
            );
            (hash.clone(), format!("skiff-service-unit-v1:sha256:{hash}"))
        }
    };
    let unit_path = format!(
        "units/services/{}/{}.json",
        publication_storage_segment(service_id, "test service id")
            .expect("test service id should project"),
        unit_hash
    );
    write_json(root, &unit_path, &value);
    ServiceUnitArtifactRef {
        schema_version: "skiff-service-unit-v1".to_string(),
        unit_identity,
        unit_hash,
        unit_path,
    }
}

pub(super) fn write_package_unit_ref(
    root: &Path,
    source_path: &str,
) -> Option<PackageUnitArtifactRef> {
    let value = read_json(root, source_path);
    write_package_unit_value_ref(root, value)
}

pub(crate) fn write_package_unit_value_ref(
    root: &Path,
    value: Value,
) -> Option<PackageUnitArtifactRef> {
    let unit_hash = package_unit_content_hash(&value).ok()?;
    let package_id = value.get("packageId")?.as_str()?.to_string();
    let version = value.get("version")?.as_str()?.to_string();
    let parsed = serde_json::from_value::<PackageUnit>(value.clone()).ok();
    let build_identity = parsed
        .as_ref()
        .and_then(|unit| package_build_identity(unit).ok())
        .or_else(|| value.get("buildIdentity")?.as_str().map(str::to_string))?;
    let abi_identity = parsed
        .as_ref()
        .and_then(|unit| package_local_abi_identity(unit).ok())
        .or_else(|| value.get("abiIdentity")?.as_str().map(str::to_string))?;
    let unit_path = format!(
        "units/packages/{}/{}.json",
        publication_storage_segment(&package_id, "test package id").ok()?,
        unit_hash
    );
    write_json(root, &unit_path, &value);
    Some(PackageUnitArtifactRef {
        schema_version: "skiff-package-unit-v1".to_string(),
        package_id,
        version,
        build_identity,
        abi_identity,
        unit_hash,
        unit_path,
    })
}

pub(super) fn write_service_assembly_ref(
    root: &Path,
    service_id: &str,
    revision_id: &str,
    protocol_identity: &str,
    service_unit: &ServiceUnitArtifactRef,
    config_shape: Option<Value>,
) -> ServiceAssemblyArtifactRef {
    let mut assembly = json!({
        "schemaVersion": "skiff-assembly-v1",
        "kind": "service",
        "service": {
            "id": service_id,
            "revisionId": revision_id,
            "protocolIdentity": protocol_identity,
            "api": null
        },
        "serviceUnit": service_unit,
        "files": [],
        "operations": [],
        "gateway": {}
    });
    if let Some(config_shape) = config_shape {
        assembly["configShape"] = config_shape;
    }
    let assembly_identity = service_assembly_identity(&assembly)
        .expect("test service assembly identity should compute");
    assembly["service"]["assemblyIdentity"] = json!(assembly_identity);
    let assembly_path = service_assembly_path(service_id, &assembly_identity);
    write_json(root, &assembly_path, &assembly);
    ServiceAssemblyArtifactRef {
        assembly_identity,
        assembly_path,
    }
}

pub(super) fn service_assembly_path(service_id: &str, assembly_identity: &str) -> String {
    let hash = assembly_identity
        .rsplit_once(":sha256:")
        .expect("test assembly identity should contain sha256")
        .1;
    format!(
        "assemblies/services/{}/{}.json",
        publication_storage_segment(service_id, "test service id")
            .expect("test service id should project"),
        hash
    )
}

pub(super) fn read_json(root: &Path, relative_path: &str) -> Value {
    serde_json::from_slice(
        &fs::read(root.join(relative_path)).expect("test artifact should be readable"),
    )
    .expect("test artifact should be valid JSON")
}

fn write_json(root: &Path, relative_path: &str, value: &Value) {
    let path = root.join(relative_path);
    fs::create_dir_all(
        path.parent()
            .expect("test artifact path should have parent"),
    )
    .expect("test artifact directory should be created");
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("test artifact should serialize"),
    )
    .expect("test artifact should be written");
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
