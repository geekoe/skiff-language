use mongodb::{
    bson::doc,
    options::{Collation, IndexOptions},
    IndexModel,
};

use super::{ensure_resume_commitment, validate_target_indexes};
use crate::index::canonical_managed_index_model;
use crate::migration_tool::MigrationToolError;

#[test]
fn resume_accepts_identical_document_and_rejects_id_collision() {
    let mapping_id = "m-00000000000000000000000000000000";
    ensure_resume_commitment(mapping_id, b"same", b"same")
        .expect("identical staged document must be resumable");
    let error = ensure_resume_commitment(mapping_id, b"existing", b"different")
        .expect_err("same _id with different content must fail closed");
    assert!(matches!(error, MigrationToolError::DuplicateId(id) if id == mapping_id));
}

#[test]
fn staging_catalog_must_exactly_match_the_final_declared_indexes() {
    let expected = canonical_managed_index_model(
        "example.test/package",
        "Item",
        "byOwner",
        vec![("ownerId".to_owned(), 1), ("createdAt".to_owned(), -1)],
        true,
    )
    .expect("canonical index");
    let primary = IndexModel::builder()
        .keys(doc! { "_id": 1 })
        .options(IndexOptions::builder().name("_id_".to_owned()).build())
        .build();
    validate_target_indexes(
        &[primary.clone(), expected.clone()],
        std::slice::from_ref(&expected),
        "mapping",
    )
    .expect("exact final indexes");

    let mut drift = expected.clone();
    drift.options.as_mut().expect("options").collation =
        Some(Collation::builder().locale("en").build());
    assert!(validate_target_indexes(
        &[primary.clone(), drift],
        std::slice::from_ref(&expected),
        "mapping"
    )
    .is_err());

    let unmanaged = IndexModel::builder()
        .keys(doc! { "operator": 1 })
        .options(IndexOptions::builder().name("operator".to_owned()).build())
        .build();
    assert!(validate_target_indexes(
        &[primary, expected.clone(), unmanaged],
        &[expected],
        "mapping"
    )
    .is_err());
}
