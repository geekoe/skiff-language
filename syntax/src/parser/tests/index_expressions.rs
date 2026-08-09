use crate::{
    ast::{BinaryOp, CallArg, Expr, Stmt},
    parser::parse_source,
};

fn assert_identifier(expr: &Expr, expected: &str) {
    assert_eq!(expr, &Expr::Identifier(expected.to_string()));
}

fn assert_named_index(expr: &Expr, expected_object: &str, expected_index: &str) {
    let Expr::Index { object, index } = expr else {
        panic!("expected index expression, got {expr:?}");
    };
    assert_identifier(object, expected_object);
    assert_identifier(index, expected_index);
}

#[test]
fn array_and_map_reads_share_the_index_expression_shape() {
    let source = parse_source(
        r#"
function read(
  items: Array<number>,
  matrix: Array<Array<number>>,
  values: Map<string, number>,
  position: integer,
  key: string,
) -> number {
  let item = items[position]
  let mapped = values[key]
  let nested = matrix[position][position]
  return item + mapped
}
"#,
    )
    .expect("array and map index reads should parse");

    let statements = &source.functions[0].body.statements;
    let Stmt::Let { value: item, .. } = &statements[0] else {
        panic!("expected indexed array binding");
    };
    assert_named_index(item, "items", "position");

    let Stmt::Let { value: mapped, .. } = &statements[1] else {
        panic!("expected indexed map binding");
    };
    assert_named_index(mapped, "values", "key");

    let Stmt::Let { value: nested, .. } = &statements[2] else {
        panic!("expected chained index binding");
    };
    let Expr::Index { object, index } = nested else {
        panic!("expected outer chained index");
    };
    assert_identifier(index, "position");
    assert_named_index(object, "matrix", "position");
}

#[test]
fn index_member_and_call_suffixes_chain_before_binary_operators() {
    let source = parse_source(
        r#"
function read() -> number {
  return source[first].field(second)[third].finish() + fallback[fourth]
}
"#,
    )
    .expect("mixed postfix index chain should parse");
    let [Stmt::Return(Some(Expr::Binary {
        op: BinaryOp::Add,
        left,
        right,
    }))] = source.functions[0].body.statements.as_slice()
    else {
        panic!("expected an addition of postfix expressions");
    };

    assert_named_index(right, "fallback", "fourth");

    let Expr::Call {
        callee: finish,
        args: finish_args,
    } = left.as_ref()
    else {
        panic!("expected final call suffix");
    };
    assert!(finish_args.is_empty());
    let Expr::Field {
        object: third_index,
        field,
    } = finish.as_ref()
    else {
        panic!("expected member suffix before final call");
    };
    assert_eq!(field, "finish");
    let Expr::Index {
        object: field_call,
        index: third,
    } = third_index.as_ref()
    else {
        panic!("expected chained index suffix");
    };
    assert_identifier(third, "third");
    let Expr::Call {
        callee: field_callee,
        args: field_args,
    } = field_call.as_ref()
    else {
        panic!("expected call suffix before chained index");
    };
    assert!(matches!(
        field_args.as_slice(),
        [CallArg::Value(Expr::Identifier(name))] if name == "second"
    ));
    let Expr::Field {
        object: first_index,
        field,
    } = field_callee.as_ref()
    else {
        panic!("expected member suffix after first index");
    };
    assert_eq!(field, "field");
    assert_named_index(first_index, "source", "first");
}

#[test]
fn assignment_and_inout_arguments_reuse_index_place_expressions() {
    let source = parse_source(
        r#"
type Box { value: number }

function mutate(inout value: number) -> void {
  value = value
}

function run(rows: Map<string, Box>, key: string) -> void {
  var writable = rows
  writable[key].value = 1
  mutate(inout writable[key].value)
}
"#,
    )
    .expect("indexed assignment and inout places should parse");

    let statements = &source.functions[1].body.statements;
    let Stmt::Assign { target, .. } = &statements[1] else {
        panic!("expected indexed assignment");
    };
    let Expr::Field { object, field } = target else {
        panic!("expected field rooted at an index place");
    };
    assert_eq!(field, "value");
    assert_named_index(object, "writable", "key");

    let Stmt::Expr(Expr::Call { args, .. }) = &statements[2] else {
        panic!("expected inout call");
    };
    let [CallArg::InOutPlace { expr }] = args.as_slice() else {
        panic!("expected one inout place argument");
    };
    let Expr::Field { object, field } = expr else {
        panic!("expected inout field rooted at an index place");
    };
    assert_eq!(field, "value");
    assert_named_index(object, "writable", "key");
}

#[test]
fn index_expression_has_one_required_serde_shape() {
    let expression = Expr::Index {
        object: Box::new(Expr::Identifier("items".to_string())),
        index: Box::new(Expr::Identifier("position".to_string())),
    };
    let wire = serde_json::to_value(&expression).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({
            "Index": {
                "object": { "Identifier": "items" },
                "index": { "Identifier": "position" }
            }
        })
    );
    assert_eq!(serde_json::from_value::<Expr>(wire).unwrap(), expression);
}
