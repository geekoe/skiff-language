use std::collections::BTreeSet;

use crate::{shared::prelude_registry::prelude_registry, shared::type_expr::TypeExpr};

pub(super) fn collect_std_type_name_import_violations(
    path: &str,
    raw: &str,
    _imported_std_roots: &BTreeSet<&str>,
    publication_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let ty = TypeExpr::parse_lossy(raw);
    ty.for_each_named_outside_function_types(|ty| {
        collect_std_type_root_import_violations(
            path,
            ty,
            _imported_std_roots,
            publication_type_names,
            violations,
        );
    });
}

fn collect_std_type_root_import_violations(
    path: &str,
    ty: &str,
    _imported_std_roots: &BTreeSet<&str>,
    publication_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let registry = prelude_registry();
    if let Some((root, _bare)) = qualified_std_type_parts(ty) {
        let allowed_roots = registry.root_projection_roots("std");
        if !allowed_roots.contains(root) {
            violations.push(format!(
                "{path}: std.{root} is not permitted as a std type module root"
            ));
        } else if registry.known_type_symbol(ty).is_none() {
            violations.push(format!("{path}: unknown standard_library type {ty}"));
        }
        return;
    }

    if publication_type_names.contains(ty) || registry.is_prelude_type_name(ty) {
        return;
    }
}

fn qualified_std_type_parts(ty: &str) -> Option<(&str, &str)> {
    let rest = ty.strip_prefix("std.")?;
    let (root, bare) = rest.split_once('.')?;
    if root.is_empty() || bare.is_empty() || bare.contains('.') {
        return None;
    }
    Some((root, bare))
}
