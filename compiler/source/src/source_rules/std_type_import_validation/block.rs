use std::collections::BTreeSet;

use crate::{shared::ast::Block, shared::ast_utils::AstVisitor};

use super::expr::StdTypeImportCollector;

pub(super) fn collect_block_std_type_import_violations(
    path: &str,
    block: &Block,
    imported_std_roots: &BTreeSet<&str>,
    publication_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let mut collector =
        StdTypeImportCollector::new(path, imported_std_roots, publication_type_names, violations);
    collector.visit_block(block);
}
