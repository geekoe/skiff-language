use std::collections::BTreeSet;

use crate::{
    shared::ast::{Expr, MatchArm, SourceFile},
    shared::ast_utils::{walk_expr, AstVisitor},
    shared::prelude_registry::prelude_registry,
};

pub fn collect_std_root_projection_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    collect_std_root_projection_violations_with_implicit_roots(path, ast, &[], violations);
}

pub fn collect_std_root_projection_violations_with_implicit_roots(
    path: &str,
    ast: &SourceFile,
    implicit_imported_roots: &[String],
    violations: &mut Vec<String>,
) {
    collect_root_projection_violations_with_implicit_roots(
        "std",
        path,
        ast,
        implicit_imported_roots,
        violations,
    );
}

fn collect_root_projection_violations_with_implicit_roots(
    root_name: &str,
    path: &str,
    ast: &SourceFile,
    implicit_imported_roots: &[String],
    violations: &mut Vec<String>,
) {
    let mut available_roots = prelude_registry().root_projection_roots(root_name);
    if available_roots.is_empty() {
        violations.push(format!(
            "{path}: {root_name} root projections are unavailable; check standard_library package metadata"
        ));
        return;
    }
    let mut imported_roots = available_roots.clone();
    imported_roots.extend(imported_roots_from_ast(ast, root_name));
    imported_roots.extend(implicit_imported_roots.iter().cloned());
    available_roots.extend(implicit_imported_roots.iter().cloned());
    let mut allowed_roots_display = available_roots
        .iter()
        .map(|root| format!("{root_name}.{root}"))
        .collect::<Vec<_>>();
    allowed_roots_display.sort();
    let allowed_roots_display = allowed_roots_display.join(", ");

    let mut collector = RootProjectionViolationWalker {
        root_name,
        path,
        allowed_roots: &available_roots,
        _imported_roots: &imported_roots,
        allowed_roots_display: &allowed_roots_display,
        violations,
    };
    for function in &ast.functions {
        collector.visit_block(&function.body);
    }
    for constant in &ast.consts {
        collector.visit_expr(&constant.value);
    }
    for impl_decl in &ast.impls {
        for method in &impl_decl.method_bodies {
            collector.visit_block(&method.body);
        }
    }
}

struct RootProjectionViolationWalker<'a, 'b> {
    root_name: &'a str,
    path: &'a str,
    allowed_roots: &'a BTreeSet<String>,
    _imported_roots: &'a BTreeSet<String>,
    allowed_roots_display: &'a str,
    violations: &'b mut Vec<String>,
}

impl AstVisitor for RootProjectionViolationWalker<'_, '_> {
    fn visit_expr(&mut self, expr: &Expr) {
        if let Some(root) = root_projection(expr, self.root_name) {
            if !self.allowed_roots.contains(root) {
                self.violations.push(format!(
                    "{}: {}.{} is not permitted as a {} module root; allowed {} module roots are {}",
                    self.path,
                    self.root_name,
                    root,
                    self.root_name,
                    self.root_name,
                    self.allowed_roots_display
                ));
            }
        }
        walk_expr(self, expr);
    }

    fn visit_match_arm(&mut self, arm: &MatchArm) {
        self.visit_block(&arm.body);
    }
}

fn root_projection<'a>(expr: &'a Expr, root_name: &str) -> Option<&'a str> {
    match expr {
        Expr::Field { object, field } => match object.as_ref() {
            Expr::Identifier(root) if root == root_name => Some(field),
            _ => root_projection(object, root_name),
        },
        Expr::Call { callee, .. } | Expr::Generic { callee, .. } => {
            root_projection(callee, root_name)
        }
        _ => None,
    }
}

fn imported_roots_from_ast(ast: &SourceFile, root_name: &str) -> BTreeSet<String> {
    ast.imports
        .iter()
        .filter_map(|import| match import.path.as_slice() {
            [root, module] if root == root_name => Some(module.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
