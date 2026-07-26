use std::collections::{BTreeMap, BTreeSet};

/// Resolves the exact collection names owned by one dependency edge.
///
/// Missing entries retain their source name. The returned map is ordered by
/// source name, so authoring map order never becomes an identity fact.
pub fn resolve_dependency_collection_names(
    source_collections: &BTreeSet<String>,
    mapping: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    validate_dependency_collection_name_mapping(mapping)?;
    for source in mapping.keys() {
        if !source_collections.contains(source) {
            return Err(format!(
                "collection mapping source {source:?} is not declared by the dependency package"
            ));
        }
    }

    let mut targets = BTreeMap::new();
    let mut target_owners = BTreeMap::new();
    for source in source_collections {
        let target = mapping
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.clone());
        if let Some(first_source) = target_owners.insert(target.clone(), source.clone()) {
            return Err(format!(
                "dependency collections {first_source:?} and {source:?} both resolve to target {target:?}"
            ));
        }
        targets.insert(source.clone(), target);
    }
    Ok(targets)
}

/// Validates facts that are self-contained in the dependency edge.
pub fn validate_dependency_collection_name_mapping(
    mapping: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut target_owners = BTreeMap::new();
    for (source, target) in mapping {
        if source.trim().is_empty() {
            return Err("collection mapping source must not be empty".to_string());
        }
        if target.trim().is_empty() {
            return Err(format!(
                "collection mapping target for source {source:?} must not be empty"
            ));
        }
        if let Some(first_source) = target_owners.insert(target, source) {
            return Err(format!(
                "collection mapping sources {first_source:?} and {source:?} both name target {target:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_empty_mapping_have_one_canonical_projection() {
        let sources = BTreeSet::from(["a".to_string(), "b".to_string()]);
        let expected = BTreeMap::from([
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "b".to_string()),
        ]);

        assert_eq!(
            resolve_dependency_collection_names(&sources, &BTreeMap::new()).unwrap(),
            expected
        );
    }

    #[test]
    fn mapping_is_exact_and_rejects_unknown_or_colliding_sources() {
        let sources = BTreeSet::from(["a".to_string(), "b".to_string()]);
        assert_eq!(
            resolve_dependency_collection_names(
                &sources,
                &BTreeMap::from([
                    ("b".to_string(), "mapped_b".to_string()),
                    ("a".to_string(), "mapped_a".to_string()),
                ]),
            )
            .unwrap(),
            BTreeMap::from([
                ("a".to_string(), "mapped_a".to_string()),
                ("b".to_string(), "mapped_b".to_string()),
            ])
        );

        assert!(resolve_dependency_collection_names(
            &sources,
            &BTreeMap::from([("missing".to_string(), "target".to_string())]),
        )
        .unwrap_err()
        .contains("is not declared"));
        assert!(resolve_dependency_collection_names(
            &sources,
            &BTreeMap::from([("a".to_string(), "b".to_string())]),
        )
        .unwrap_err()
        .contains("both resolve"));
        assert!(
            validate_dependency_collection_name_mapping(&BTreeMap::from([
                ("a".to_string(), "target".to_string()),
                ("b".to_string(), "target".to_string()),
            ]))
            .unwrap_err()
            .contains("both name target")
        );
    }
}
