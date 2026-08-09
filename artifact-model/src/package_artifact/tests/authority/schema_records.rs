use std::collections::BTreeMap;

use serde::Deserialize;

use super::*;
use crate::{
    derive_package_schema_type_id, validate_bytecode_schema_records,
    validate_package_build_authority, ContractTypeDescriptor, ContractTypeRef,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord,
};

fn representation(name: &str) -> PackageSchemaCanonicalDescriptor {
    PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Representation {
            target: ContractTypeRef::builtin(name),
        },
    }
}

fn record(
    package_id: &str,
    stable_schema_key: &str,
    descriptor: PackageSchemaCanonicalDescriptor,
) -> (PackageSchemaTypeId, PackageSchemaTypeRecord) {
    let type_id =
        derive_package_schema_type_id(package_id, stable_schema_key, &descriptor).unwrap();
    (
        type_id.clone(),
        PackageSchemaTypeRecord {
            package_id: package_id.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: type_id,
            canonical_descriptor: descriptor,
        },
    )
}

#[test]
fn package_schema_v2_identity_matches_the_existing_authority_golden() {
    let type_id =
        derive_package_schema_type_id(PACKAGE_ID, "User", &representation("string")).unwrap();
    assert_eq!(
        type_id.as_str(),
        "skiff-package-schema-type-v2:sha256:c72c69aa175ea061ff4bd0a88aac3f83b1c064995cdddcaf47ae05ba1dbf6c31"
    );
}

#[test]
fn owner_local_descriptor_graph_is_exact_and_external_children_are_not_copied() {
    let (leaf_id, leaf) = record(PACKAGE_ID, "Leaf", representation("string"));
    let parent_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([
                (
                    "external".to_string(),
                    ContractTypeRef::package_schema(
                        "dependency.pkg",
                        "External",
                        PackageSchemaTypeId::new("dependency-type"),
                    ),
                ),
                (
                    "leaf".to_string(),
                    ContractTypeRef::package_schema(PACKAGE_ID, "Leaf", leaf_id.clone()),
                ),
            ]),
        },
    };
    let (parent_id, parent) = record(PACKAGE_ID, "Parent", parent_descriptor);
    let records = BTreeMap::from([(leaf_id, leaf), (parent_id, parent)]);
    assert!(validate_bytecode_schema_records(PACKAGE_ID, &records).is_ok());

    let mut artifact = authority_artifact();
    artifact.bytecode_schema_records = records;
    assert!(validate_package_build_authority(&artifact).is_ok());
}

#[test]
fn foreign_rows_missing_local_children_and_identity_drift_fail_closed() {
    let (foreign_id, foreign) = record("dependency.pkg", "Foreign", representation("string"));
    assert!(
        validate_bytecode_schema_records(PACKAGE_ID, &BTreeMap::from([(foreign_id, foreign)]))
            .is_err()
    );

    let missing_id = PackageSchemaTypeId::new("missing-local-child");
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Alias {
            target: ContractTypeRef::package_schema(PACKAGE_ID, "Missing", missing_id),
        },
    };
    let (owner_id, owner) = record(PACKAGE_ID, "Owner", descriptor);
    assert!(
        validate_bytecode_schema_records(PACKAGE_ID, &BTreeMap::from([(owner_id, owner)])).is_err()
    );

    let (type_id, mut drifted) = record(PACKAGE_ID, "User", representation("string"));
    drifted.canonical_descriptor = representation("integer");
    assert!(
        validate_bytecode_schema_records(PACKAGE_ID, &BTreeMap::from([(type_id, drifted)]))
            .is_err()
    );

    let (leaf_id, leaf) = record(PACKAGE_ID, "Leaf", representation("string"));
    let wrong_child_key = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Alias {
            target: ContractTypeRef::package_schema(PACKAGE_ID, "Wrong", leaf_id.clone()),
        },
    };
    let (owner_id, owner) = record(PACKAGE_ID, "Owner", wrong_child_key);
    assert!(validate_bytecode_schema_records(
        PACKAGE_ID,
        &BTreeMap::from([(leaf_id, leaf), (owner_id, owner)])
    )
    .is_err());

    let (first_id, first) = record(PACKAGE_ID, "Duplicate", representation("string"));
    let (second_id, second) = record(PACKAGE_ID, "Duplicate", representation("integer"));
    assert!(validate_bytecode_schema_records(
        PACKAGE_ID,
        &BTreeMap::from([(first_id, first), (second_id, second)])
    )
    .is_err());
}

#[test]
fn local_cycles_noncanonical_descriptors_and_depth_exhaustion_fail_closed() {
    let a = PackageSchemaTypeId::new("forged-a");
    let b = PackageSchemaTypeId::new("forged-b");
    let cyclic = BTreeMap::from([
        (
            a.clone(),
            PackageSchemaTypeRecord {
                package_id: PACKAGE_ID.to_string(),
                stable_schema_key: "A".to_string(),
                package_schema_type_id: a.clone(),
                canonical_descriptor: PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: ContractTypeDescriptor::Alias {
                        target: ContractTypeRef::package_schema(PACKAGE_ID, "B", b.clone()),
                    },
                },
            },
        ),
        (
            b.clone(),
            PackageSchemaTypeRecord {
                package_id: PACKAGE_ID.to_string(),
                stable_schema_key: "B".to_string(),
                package_schema_type_id: b,
                canonical_descriptor: PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: ContractTypeDescriptor::Alias {
                        target: ContractTypeRef::package_schema(PACKAGE_ID, "A", a),
                    },
                },
            },
        ),
    ]);
    let error = validate_bytecode_schema_records(PACKAGE_ID, &cyclic).unwrap_err();
    assert!(error.message().contains("cycle"));

    assert!(
        derive_package_schema_type_id(PACKAGE_ID, "Legacy", &representation("String")).is_err()
    );

    let mut nested = ContractTypeRef::builtin("string");
    for _ in 0..=crate::MAX_BYTECODE_SCHEMA_DEPTH {
        nested = ContractTypeRef::Nullable {
            inner: Box::new(nested),
        };
    }
    let forged = PackageSchemaTypeId::new("too-deep");
    let records = BTreeMap::from([(
        forged.clone(),
        PackageSchemaTypeRecord {
            package_id: PACKAGE_ID.to_string(),
            stable_schema_key: "Deep".to_string(),
            package_schema_type_id: forged,
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::Alias { target: nested },
            },
        },
    )]);
    assert!(validate_bytecode_schema_records(PACKAGE_ID, &records).is_err());
}

#[derive(Deserialize)]
struct WireRecords {
    #[serde(
        rename = "records",
        deserialize_with = "crate::package_artifact::schema_records::deserialize_bytecode_schema_records"
    )]
    _records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
}

#[test]
fn bytecode_schema_record_wire_rejects_noncanonical_or_duplicate_map_keys() {
    let (_, row) = record(PACKAGE_ID, "User", representation("string"));
    let row = serde_json::to_string(&row).unwrap();
    for wire in [
        format!(r#"{{"records":{{"z":{row},"a":{row}}}}}"#),
        format!(r#"{{"records":{{"a":{row},"a":{row}}}}}"#),
    ] {
        assert!(serde_json::from_str::<WireRecords>(&wire).is_err());
    }

    let (type_id, row) = record(PACKAGE_ID, "Wire", representation("string"));
    let mut artifact = authority_artifact();
    artifact
        .bytecode_schema_records
        .insert(type_id.clone(), row);
    let mut wire = serde_json::to_value(artifact).unwrap();
    wire["bytecodeSchemaRecords"][type_id.as_str()]["stableSchemaKey"] = serde_json::json!("Drift");
    assert!(serde_json::from_value::<PackageArtifact>(wire).is_err());
}
