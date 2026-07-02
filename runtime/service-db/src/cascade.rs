use std::collections::BTreeSet;

use mongodb::bson::{Bson, Document};
use serde_json::Value;
use skiff_runtime_capability_context::ServiceDbChange;

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct CascadeFileDeletePlan {
    pub file_ids: Vec<String>,
}

pub fn cascade_plan_for_deleted_documents(
    documents: &[Document],
    paths: &[Vec<String>],
) -> CascadeFileDeletePlan {
    let mut ids = BTreeSet::new();
    for document in documents {
        for path in paths {
            collect_file_ids_at_path(document, path, &mut ids);
        }
    }
    CascadeFileDeletePlan {
        file_ids: ids.into_iter().collect(),
    }
}

pub fn cascade_plan_for_replacement(
    old: &Document,
    replacement: &Document,
    paths: &[Vec<String>],
) -> CascadeFileDeletePlan {
    let mut ids = BTreeSet::new();
    for path in paths {
        let old_id = file_id_at_path(old, path);
        let new_id = file_id_at_path(replacement, path);
        if old_id.is_some() && old_id != new_id {
            ids.insert(old_id.expect("old_id is_some"));
        }
    }
    CascadeFileDeletePlan {
        file_ids: ids.into_iter().collect(),
    }
}

pub fn cascade_plan_for_change(
    old: &Document,
    change: &ServiceDbChange,
    paths: &[Vec<String>],
) -> CascadeFileDeletePlan {
    let mut ids = BTreeSet::new();
    for path in paths {
        let old_id = file_id_at_path(old, path);
        let Some(old_id) = old_id else {
            continue;
        };
        let Some(new_id) = replacement_file_id_for_change_path(change, path) else {
            continue;
        };
        if new_id.as_deref() != Some(old_id.as_str()) {
            ids.insert(old_id);
        }
    }
    CascadeFileDeletePlan {
        file_ids: ids.into_iter().collect(),
    }
}

pub fn cascade_plan_for_changed_documents(
    documents: &[Document],
    change: &ServiceDbChange,
    paths: &[Vec<String>],
) -> CascadeFileDeletePlan {
    let mut ids = BTreeSet::new();
    for document in documents {
        ids.extend(cascade_plan_for_change(document, change, paths).file_ids);
    }
    CascadeFileDeletePlan {
        file_ids: ids.into_iter().collect(),
    }
}

fn replacement_file_id_for_change_path(
    change: &ServiceDbChange,
    path: &[String],
) -> Option<Option<String>> {
    let path_text = path.join(".");
    if change.unset_contains(&path_text)
        || change
            .unset_fields()
            .any(|field| is_ancestor_path(field, path))
    {
        return Some(None);
    }
    if let Some(value) = change.set_value(&path_text) {
        return Some(file_id_from_json(value.as_value()));
    }
    for (field, value) in change.set_entries() {
        if !is_ancestor_path(field, path) {
            continue;
        }
        let remaining = &path[field.split('.').count()..];
        return Some(file_id_from_json_at_path(value.as_value(), remaining));
    }
    None
}

fn is_ancestor_path(field: &str, path: &[String]) -> bool {
    let segments = field.split('.').collect::<Vec<_>>();
    segments.len() < path.len()
        && segments
            .iter()
            .zip(path.iter())
            .all(|(left, right)| *left == right)
}

fn collect_file_ids_at_path(document: &Document, path: &[String], ids: &mut BTreeSet<String>) {
    if let Some(id) = file_id_at_path(document, path) {
        ids.insert(id);
    }
}

fn file_id_at_path(document: &Document, path: &[String]) -> Option<String> {
    let (first, rest) = path.split_first()?;
    file_id_from_bson_at_path(document.get(first)?, rest)
}

fn file_id_from_bson_at_path(value: &Bson, path: &[String]) -> Option<String> {
    if path.is_empty() {
        return file_id_from_bson(value);
    }
    let (first, rest) = path.split_first()?;
    let document = value.as_document()?;
    file_id_from_bson_at_path(document.get(first)?, rest)
}

fn file_id_from_bson(value: &Bson) -> Option<String> {
    let document = value.as_document()?;
    document.get_str("id").ok().map(str::to_string)
}

fn file_id_from_json_at_path(value: &Value, path: &[String]) -> Option<String> {
    if path.is_empty() {
        return file_id_from_json(value);
    }
    let (first, rest) = path.split_first()?;
    file_id_from_json_at_path(value.as_object()?.get(first)?, rest)
}

fn file_id_from_json(value: &Value) -> Option<String> {
    value.get("id")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::*;

    #[test]
    fn deleted_document_plan_extracts_direct_and_optional_file_ids() {
        let documents = vec![
            doc! {
                "_id": "row-1",
                "requestFile": { "id": "file-1", "size": 12_i64, "sha256": "abc" },
                "responseFile": null,
            },
            doc! {
                "_id": "row-2",
                "requestFile": { "id": "file-2", "size": 14_i64, "sha256": "def" },
            },
        ];
        let paths = vec![
            vec!["requestFile".to_string()],
            vec!["responseFile".to_string()],
        ];

        assert_eq!(
            cascade_plan_for_deleted_documents(&documents, &paths).file_ids,
            vec!["file-1".to_string(), "file-2".to_string()]
        );
    }

    #[test]
    fn change_plan_ignores_same_file_and_deletes_replaced_or_unset_old_file() {
        let old = doc! {
            "_id": "row-1",
            "sameFile": { "id": "file-same" },
            "replacedFile": { "id": "file-old" },
            "clearedFile": { "id": "file-clear" },
        };
        let mut change = ServiceDbChange::new();
        change.set("sameFile", serde_json::json!({ "id": "file-same" }));
        change.set("replacedFile", serde_json::json!({ "id": "file-new" }));
        change.unset("clearedFile");
        let paths = vec![
            vec!["sameFile".to_string()],
            vec!["replacedFile".to_string()],
            vec!["clearedFile".to_string()],
        ];

        assert_eq!(
            cascade_plan_for_change(&old, &change, &paths).file_ids,
            vec!["file-clear".to_string(), "file-old".to_string()]
        );
    }

    #[test]
    fn changed_documents_plan_deduplicates_only_replaced_old_files() {
        let documents = vec![
            doc! {
                "_id": "row-1",
                "file": { "id": "file-old" },
            },
            doc! {
                "_id": "row-2",
                "file": { "id": "file-old" },
            },
        ];
        let mut change = ServiceDbChange::new();
        change.set("file", serde_json::json!({ "id": "file-new" }));
        let paths = vec![vec!["file".to_string()]];

        assert_eq!(
            cascade_plan_for_changed_documents(&documents, &change, &paths).file_ids,
            vec!["file-old".to_string()]
        );
    }
}
