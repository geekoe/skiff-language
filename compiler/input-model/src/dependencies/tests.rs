use skiff_artifact_model::{
    DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS, DEPENDENCY_ALIAS_POSITIVE_VECTORS,
    DEPENDENCY_ALIAS_RESERVED_VECTORS,
};

use super::*;

#[test]
fn compiler_helpers_thinly_delegate_the_shared_dependency_alias_vectors() {
    for alias in DEPENDENCY_ALIAS_POSITIVE_VECTORS {
        assert!(is_valid_source_import_alias(alias), "{alias}");
        assert!(!is_reserved_source_import_alias(alias), "{alias}");
    }
    for alias in DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS {
        assert!(!is_valid_source_import_alias(alias), "{alias}");
    }
    for alias in DEPENDENCY_ALIAS_RESERVED_VECTORS {
        assert!(is_valid_source_import_alias(alias), "{alias}");
        assert!(is_reserved_source_import_alias(alias), "{alias}");
    }
}

#[test]
fn dependency_rejects_removed_collection_mapping_authoring() {
    let removed = serde_json::from_value::<PackageDependency>(serde_json::json!({
        "id": "example.store",
        "version": "1.0.0",
        "alias": "store",
        "collection_name_mapping": {
            "package_secret": "mapped_package_secret"
        }
    }));
    assert!(removed.is_err());
}
