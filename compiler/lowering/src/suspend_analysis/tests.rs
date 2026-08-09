use super::*;
use skiff_syntax::parser::parse_source;

#[test]
fn missing_expression_facts_do_not_invent_full_type_for_projected_db_read() {
    let ast = parse_source(
        r#"
              function projected(id: string) -> void {
                let credential = db require Credential(id) { fields { label } }
              }

              function full(id: string) -> void {
                let credential = db require Credential(id)
              }
            "#,
    )
    .expect("test source should parse");
    let package_aliases = BTreeMap::new();
    let service_dependency_aliases = BTreeSet::new();
    let resolved_call_targets = ResolvedCallTargetFacts::empty();
    let analyzer = SuspendAnalyzer::new(
        &ast,
        "test.db_projection",
        &package_aliases,
        &service_dependency_aliases,
        None,
        &resolved_call_targets,
    );
    let values = BTreeMap::new();
    let context = SuspendContext {
        analyzer: &analyzer,
        values: &values,
        env: SuspendTypeEnv::default(),
        owner: ExpressionOwnerKey::Function("projected".to_string()),
        next_expression_index: 0,
    };

    let projected = let_value(&ast.functions[0].body.statements[0]);
    assert_eq!(
        context.legacy_expr_type_for_missing_facts(projected),
        None,
        "a projected DB read cannot be typed without DB field metadata"
    );

    let full = let_value(&ast.functions[1].body.statements[0]);
    assert_eq!(
        context.legacy_expr_type_for_missing_facts(full),
        Some("Credential".to_string()),
        "legacy nominal typing remains available for an unprojected DB read"
    );
}

fn let_value(stmt: &Stmt) -> &Expr {
    let Stmt::Let { value, .. } = stmt else {
        panic!("expected let statement, got {stmt:?}");
    };
    value
}
