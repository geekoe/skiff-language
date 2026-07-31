use super::*;

fn native(name: &str) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args: Vec::new(),
    }
}

#[test]
fn projection_adds_key_and_preserves_nested_nullable_shape() {
    let fields = BTreeMap::from([
        ("id".to_string(), native("string")),
        (
            "profile".to_string(),
            TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::Record {
                    fields: BTreeMap::from([
                        ("displayName".to_string(), native("string")),
                        ("ignored".to_string(), native("number")),
                    ]),
                }),
            },
        ),
    ]);

    let projected = project_db_read_type(
        "Credential",
        "id",
        native("Credential"),
        &fields,
        Some(&[vec!["profile".to_string(), "displayName".to_string()]]),
    )
    .expect("projection should build");

    assert_eq!(
        projected,
        TypeRefIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), native("string")),
                (
                    "profile".to_string(),
                    TypeRefIr::Nullable {
                        inner: Box::new(TypeRefIr::Record {
                            fields: BTreeMap::from([
                                ("displayName".to_string(), native("string"),)
                            ]),
                        }),
                    },
                ),
            ]),
        }
    );
}

#[test]
fn projection_rejects_duplicate_and_parent_child_paths() {
    let fields = BTreeMap::from([
        ("id".to_string(), native("string")),
        (
            "profile".to_string(),
            TypeRefIr::Record {
                fields: BTreeMap::from([("displayName".to_string(), native("string"))]),
            },
        ),
    ]);

    let duplicate = project_db_read_type(
        "Credential",
        "id",
        native("Credential"),
        &fields,
        Some(&[vec!["id".to_string()], vec!["id".to_string()]]),
    )
    .expect_err("duplicate should fail");
    assert!(duplicate.contains("duplicate db projection field `id`"));

    let parent_child = project_db_read_type(
        "Credential",
        "id",
        native("Credential"),
        &fields,
        Some(&[
            vec!["profile".to_string()],
            vec!["profile".to_string(), "displayName".to_string()],
        ]),
    )
    .expect_err("parent and child should fail");
    assert!(parent_child.contains("cannot include both `profile` and child path"));
}

#[test]
fn projection_rejects_unknown_and_non_record_paths() {
    let fields = BTreeMap::from([
        ("id".to_string(), native("string")),
        (
            "profile".to_string(),
            TypeRefIr::Record {
                fields: BTreeMap::from([("displayName".to_string(), native("string"))]),
            },
        ),
    ]);

    for (path, expected) in [
        (
            vec!["missing".to_string()],
            "db projection references unknown field `missing`",
        ),
        (
            vec!["profile".to_string(), "missing".to_string()],
            "db projection references unknown field `profile.missing`",
        ),
        (
            vec!["id".to_string(), "value".to_string()],
            "db projection field `id.value` on Credential cannot traverse non-record type",
        ),
    ] {
        let error = project_db_read_type(
            "Credential",
            "id",
            native("Credential"),
            &fields,
            Some(&[path]),
        )
        .expect_err("invalid projection path should fail");
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error:?}"
        );
    }
}
