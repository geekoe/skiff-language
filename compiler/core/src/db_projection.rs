use std::collections::BTreeMap;

use skiff_artifact_model::TypeRefIr;

/// Builds the exact logical record type returned by a DB read projection.
///
/// The primary key is part of every logical projection even when the source
/// `fields` block omits it. Callers provide structurally expanded DB field
/// types so nested selection has one implementation across source typing and
/// File IR lowering.
pub fn project_db_read_type(
    db_name: &str,
    key_name: &str,
    full_target: TypeRefIr,
    field_types: &BTreeMap<String, TypeRefIr>,
    projection_paths: Option<&[Vec<String>]>,
) -> Result<TypeRefIr, String> {
    let Some(projection_paths) = projection_paths else {
        return Ok(full_target);
    };

    let mut root = ProjectionTypeNode::default();
    for path in projection_paths {
        insert_projection_type_path(&mut root, path, db_name, field_types)?;
    }
    if !projection_paths
        .iter()
        .any(|path| path.first().is_some_and(|name| name == key_name))
    {
        insert_projection_type_path(&mut root, &[key_name.to_string()], db_name, field_types)?;
    }

    Ok(TypeRefIr::Record {
        fields: root
            .children
            .into_iter()
            .map(|(name, node)| {
                let ty = field_types.get(&name).ok_or_else(|| {
                    format!("db projection references unknown field `{name}` on {db_name}")
                })?;
                Ok((
                    name.clone(),
                    projection_node_type(db_name, &name, ty, &node)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?,
    })
}

#[derive(Debug, Default)]
struct ProjectionTypeNode {
    terminal: bool,
    children: BTreeMap<String, ProjectionTypeNode>,
}

fn insert_projection_type_path(
    root: &mut ProjectionTypeNode,
    segments: &[String],
    db_name: &str,
    field_types: &BTreeMap<String, TypeRefIr>,
) -> Result<(), String> {
    let Some(first) = segments.first() else {
        return Err(format!(
            "db projection field path on {db_name} cannot be empty"
        ));
    };
    if !field_types.contains_key(first) {
        return Err(format!(
            "db projection references unknown field `{first}` on {db_name}"
        ));
    }

    let text = segments.join(".");
    let mut node = root;
    for (index, segment) in segments.iter().enumerate() {
        if node.terminal {
            let parent = segments[..index].join(".");
            return Err(format!(
                "db projection cannot include both `{parent}` and child path `{text}` on {db_name}"
            ));
        }
        node = node.children.entry(segment.clone()).or_default();
    }
    if node.terminal {
        return Err(format!(
            "duplicate db projection field `{text}` on {db_name}"
        ));
    }
    if !node.children.is_empty() {
        let child = first_projection_child_path(segments, node);
        return Err(format!(
            "db projection cannot include both `{text}` and child path `{child}` on {db_name}"
        ));
    }
    node.terminal = true;
    Ok(())
}

fn first_projection_child_path(parent: &[String], node: &ProjectionTypeNode) -> String {
    let mut path = parent.to_vec();
    let mut current = node;
    while let Some((name, next)) = current.children.iter().next() {
        path.push(name.clone());
        current = next;
    }
    path.join(".")
}

fn projection_node_type(
    db_name: &str,
    path: &str,
    ty: &TypeRefIr,
    node: &ProjectionTypeNode,
) -> Result<TypeRefIr, String> {
    if node.terminal {
        return Ok(ty.clone());
    }

    let (inner, nullable) = unwrap_nullable_type(ty);
    let TypeRefIr::Record { fields } = inner else {
        let attempted_path = first_projection_child_type_path(path, node);
        return Err(format!(
            "db projection field `{attempted_path}` on {db_name} cannot traverse non-record type"
        ));
    };
    let projected = TypeRefIr::Record {
        fields: node
            .children
            .iter()
            .map(|(name, child)| {
                let child_path = format!("{path}.{name}");
                let child_ty = fields.get(name).ok_or_else(|| {
                    format!("db projection references unknown field `{child_path}` on {db_name}")
                })?;
                Ok((
                    name.clone(),
                    projection_node_type(db_name, &child_path, child_ty, child)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?,
    };
    if nullable {
        Ok(TypeRefIr::Nullable {
            inner: Box::new(projected),
        })
    } else {
        Ok(projected)
    }
}

fn first_projection_child_type_path(parent: &str, node: &ProjectionTypeNode) -> String {
    let mut path = vec![parent.to_string()];
    let mut current = node;
    while let Some((name, next)) = current.children.iter().next() {
        path.push(name.clone());
        current = next;
    }
    path.join(".")
}

fn unwrap_nullable_type(ty: &TypeRefIr) -> (&TypeRefIr, bool) {
    match ty {
        TypeRefIr::Nullable { inner } => (inner, true),
        _ => (ty, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(name: &str) -> TypeRefIr {
        TypeRefIr::Native {
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
                                fields: BTreeMap::from([(
                                    "displayName".to_string(),
                                    native("string"),
                                )]),
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
}
