use crate::shared::ast::Block;
use crate::shared::ast_utils::collect_reserved_binding_violations;

use super::*;

pub(super) fn validate_package_reserved_roots_in_block(
    path: &str,
    block: &Block,
    violations: &mut Vec<String>,
) {
    collect_reserved_binding_violations(path, block, violations, is_reserved_root);
}

pub(super) fn is_reserved_root(name: &str) -> bool {
    name == "std" || name == "ext" || name == "connect" || name == "config" || name == "root"
}

#[cfg(test)]
mod tests;
