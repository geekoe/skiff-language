use std::collections::BTreeMap;

use skiff_artifact_model::{
    ContractTypeDescriptor, LiteralIr, PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord,
};

use super::*;
use crate::{FileAddr, UnitAddr};

fn addr(slot: usize, type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Package(slot),
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

fn link(package_id: &str, stable_key: &str, type_id: &str, addr: TypeAddr) -> ServiceErrorTypeLink {
    let identity =
        ServiceErrorPublicIdentity::new(package_id, stable_key, PackageSchemaTypeId::new(type_id));
    let record = Arc::new(PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: PackageSchemaTypeId::new(type_id),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
        },
    });
    ServiceErrorTypeLink::try_new(
        identity,
        record,
        ServiceErrorExecutionContext::Declaration {
            addr,
            kind: ServiceErrorDeclarationKind::Record,
        },
    )
    .unwrap()
}

#[test]
fn exact_identity_can_have_multiple_execution_addresses() {
    let index = ServiceErrorTypeIndex::try_new([
        link("example/errors", "Fault", "type:fault", addr(0, 0)),
        link("example/errors", "Fault", "type:fault", addr(1, 0)),
    ])
    .unwrap();
    let identity = ServiceErrorPublicIdentity::new(
        "example/errors",
        "Fault",
        PackageSchemaTypeId::new("type:fault"),
    );
    assert_eq!(index.by_public_identity(&identity).unwrap().len(), 2);
    assert_eq!(
        index
            .execution_addresses_by_public_identity(&identity)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn one_execution_address_cannot_claim_two_public_identities() {
    let error = ServiceErrorTypeIndex::try_new([
        link("example/a", "Fault", "type:a", addr(0, 0)),
        link("example/b", "Fault", "type:b", addr(0, 0)),
    ])
    .unwrap_err();
    assert!(matches!(
        error,
        ServiceErrorTypeIndexError::ExecutionAddressIdentityConflict { .. }
    ));
}

#[test]
fn equal_path_and_shape_under_different_owners_do_not_merge() {
    let index = ServiceErrorTypeIndex::try_new([
        link("example/a", "Fault", "type:a", addr(0, 0)),
        link("example/b", "Fault", "type:b", addr(1, 0)),
    ])
    .unwrap();
    assert_eq!(index.public_identity_len(), 2);
}

#[test]
fn same_type_id_with_conflicting_record_content_fails_closed() {
    let first = link("example/errors", "Fault", "type:fault", addr(0, 0));
    let mut second = link("example/errors", "Fault", "type:fault", addr(1, 0));
    Arc::make_mut(&mut second.record)
        .canonical_descriptor
        .descriptor = ContractTypeDescriptor::Enumeration {
        variants: vec!["different".to_string()],
    };
    assert!(matches!(
        ServiceErrorTypeIndex::try_new([first, second]).unwrap_err(),
        ServiceErrorTypeIndexError::TypeRecordConflict { .. }
    ));
}

#[test]
fn named_union_branches_keep_exact_context_on_one_execution_address() {
    let identity = ServiceErrorPublicIdentity::new(
        "example/errors",
        "Fault",
        PackageSchemaTypeId::new("type:fault"),
    );
    let record = Arc::new(PackageSchemaTypeRecord {
        package_id: "example/errors".to_string(),
        stable_schema_key: "Fault".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("type:fault"),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Enumeration {
                variants: vec!["left".to_string(), "right".to_string()],
            },
        },
    });
    let union_addr = addr(0, 0);
    let links = ["left", "right"]
        .into_iter()
        .enumerate()
        .map(|(branch_index, value)| {
            ServiceErrorTypeLink::try_new(
                identity.clone(),
                Arc::clone(&record),
                ServiceErrorExecutionContext::NamedUnionBranch {
                    union_addr: union_addr.clone(),
                    branch_index,
                    branch: LinkedNamedUnionBranch::Literal {
                        value: LiteralIr::String {
                            value: value.to_string(),
                        },
                    },
                    representation_owner: None,
                },
            )
            .unwrap()
        });
    let index = ServiceErrorTypeIndex::try_new(links).unwrap();
    assert_eq!(index.by_public_identity(&identity).unwrap().len(), 2);
    assert!(index
        .by_execution(&ServiceErrorExecutionKey::NamedUnionBranch {
            union_addr,
            branch_index: 1,
        })
        .is_some());
}
