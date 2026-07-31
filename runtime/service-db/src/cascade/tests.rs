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
