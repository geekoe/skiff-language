use skiff_runtime_linked_program::ExprRefIr;

use super::unsupported_linked_index_expression;
use crate::error::RuntimeError;

#[test]
fn linked_index_expression_fails_closed_without_legacy_execution() {
    let error = unsupported_linked_index_expression(
        &ExprRefIr { expression: 0 },
        &ExprRefIr { expression: 1 },
    )
    .expect_err("legacy linked index evaluation must fail closed");

    assert!(matches!(
        error,
        RuntimeError::Unsupported(message)
            if message == "linked index expressions require bytecode execution"
    ));
}
