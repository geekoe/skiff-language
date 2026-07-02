use std::collections::BTreeSet;

use crate::{
    shared::ast::{Expr, Pattern, TypeRef},
    shared::ast_utils::{walk_expr, walk_pattern, AstVisitor},
};

use super::refs::collect_std_type_name_import_violations;

pub(super) struct StdTypeImportCollector<'a> {
    path: &'a str,
    imported_std_roots: &'a BTreeSet<&'a str>,
    publication_type_names: &'a BTreeSet<String>,
    violations: &'a mut Vec<String>,
}

impl<'a> StdTypeImportCollector<'a> {
    pub(super) fn new(
        path: &'a str,
        imported_std_roots: &'a BTreeSet<&'a str>,
        publication_type_names: &'a BTreeSet<String>,
        violations: &'a mut Vec<String>,
    ) -> Self {
        Self {
            path,
            imported_std_roots,
            publication_type_names,
            violations,
        }
    }

    fn collect_type_name(&mut self, name: &str) {
        collect_std_type_name_import_violations(
            self.path,
            name,
            self.imported_std_roots,
            self.publication_type_names,
            self.violations,
        );
    }
}

impl AstVisitor for StdTypeImportCollector<'_> {
    fn visit_pattern(&mut self, pattern: &Pattern) {
        if let Pattern::Nominal { name, .. } = pattern {
            self.collect_type_name(name);
        }
        walk_pattern(self, pattern);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Generic { callee, type_args } => {
                for type_arg in type_args {
                    self.visit_type_ref(type_arg);
                }
                self.visit_expr(callee);
            }
            Expr::Record {
                type_name,
                type_args,
                fields,
            } => {
                self.collect_type_name(type_name);
                for type_arg in type_args {
                    self.visit_type_ref(type_arg);
                }
                for (_, value) in fields {
                    self.visit_expr(value);
                }
            }
            _ => walk_expr(self, expr),
        }
    }

    fn visit_type_ref(&mut self, ty: &TypeRef) {
        self.collect_type_name(&ty.name);
    }
}
