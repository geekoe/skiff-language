//! Phase 0 baseline: precise tolerant-mode recovery behavior.
//!
//! `parse_callable_decl_body_tolerant` restores `current` to
//! the body start and then `skip_balanced_block` consumes the whole failed
//! body, so the cursor must land exactly on the token after the closing brace.

use crate::parser::parse_source_with_bodies_tolerant;

#[test]
fn tolerant_top_level_body_failure_resumes_after_skipped_balanced_block() {
    let source = "function broken() -> number { assert true }function ok() -> number { return 2 }";
    let ast = parse_source_with_bodies_tolerant(source).unwrap();

    assert!(ast
        .function_signatures
        .iter()
        .any(|signature| signature.name == "broken"));
    assert_eq!(ast.function_signatures.len(), 1);

    assert_eq!(ast.functions.len(), 1);
    assert_eq!(ast.functions[0].name, "ok");
    // Cursor resumes immediately after the failed body's closing brace.
    let broken_close = source.find('}').unwrap();
    assert_eq!(ast.functions[0].span.start.offset, broken_close + 1);
    assert!(matches!(
        &ast.functions[0].body.statements[0],
        crate::ast::Stmt::Return(Some(crate::ast::Expr::Literal(
            crate::ast::Literal::Number(value)
        ))) if *value == 2.0
    ));
}

#[test]
fn tolerant_body_failure_swallows_nested_declaration_tokens() {
    let source = "function broken() -> number { assert true function nested() -> number { return 1 } }function ok() -> number { return 2 }";
    let ast = parse_source_with_bodies_tolerant(source).unwrap();

    assert_eq!(ast.function_signatures.len(), 1);
    assert!(ast
        .function_signatures
        .iter()
        .any(|signature| signature.name == "broken"));
    assert_eq!(ast.functions.len(), 1);
    assert_eq!(ast.functions[0].name, "ok");
    // The nested `function`/`return` tokens lived inside the skipped balanced
    // block and must never be re-read as top-level declarations.
    assert!(!ast
        .functions
        .iter()
        .any(|function| function.name == "nested"));
}

#[test]
fn tolerant_impl_method_failure_resumes_after_method_block() {
    let source =
        "impl Example { function broken() -> number { assert true }function ok() -> number { return 2 } }";
    let ast = parse_source_with_bodies_tolerant(source).unwrap();
    let implementation = &ast.impls[0];

    assert_eq!(implementation.methods.len(), 2);
    assert!(implementation
        .methods
        .iter()
        .any(|method| method.name == "broken"));
    assert!(implementation
        .methods
        .iter()
        .any(|method| method.name == "ok"));

    assert_eq!(implementation.method_bodies.len(), 1);
    assert_eq!(implementation.method_bodies[0].name, "ok");
    let broken_close = source.find("}function ok").unwrap();
    assert_eq!(
        implementation.method_bodies[0].span.start.offset,
        broken_close + 1
    );

    // The span side channel only keeps successfully recovered bodies.
    assert_eq!(ast.source_spans.impl_methods.len(), 1);
    assert_eq!(ast.source_spans.impl_methods[0].body.statements.len(), 1);
}

#[test]
fn tolerant_signature_failure_is_not_recoverable() {
    let source = "function broken() -> { assert true }function ok() -> number { return 2 }";
    let error = parse_source_with_bodies_tolerant(source)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("expected symbol :"),
        "unexpected signature failure: {error}"
    );
}

#[test]
fn tolerant_provider_failure_is_a_hard_error_at_the_provider_token() {
    // `provider function` starts a callable signature (`check_function_start`
    // includes provider), so the modifier error fires at 1:1 before any body
    // parsing; tolerant mode cannot recover it.
    let top_level = "provider function hostValue() -> string { return \"x\" }";
    let error = parse_source_with_bodies_tolerant(top_level)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "legacy provider syntax has been removed; use native std APIs or package APIs instead at 1:1"
    );

    // A capability declaration (`provider app.live`) hits the legacy-provider
    // error at the provider token (existing tests.rs:1069 covers the message;
    // this locks the exact position).
    let capability = "provider app.live";
    let error = parse_source_with_bodies_tolerant(capability)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "legacy provider syntax has been removed; use native std APIs or package APIs instead at 1:1"
    );

    // Inside an impl, `provider` starts a function and the modifier error fires
    // at the provider token itself; this is a hard abort, not a body fallback.
    let impl_method = "impl Example { provider function hostValue() -> string { return \"x\" } }";
    let error = parse_source_with_bodies_tolerant(impl_method)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        format!(
            "legacy provider syntax has been removed; use native std APIs or package APIs instead at 1:{}",
            impl_method.find("provider").unwrap() + 1
        )
    );
}
