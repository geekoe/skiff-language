use skiff_syntax::parser::parse_source;

use super::*;

#[test]
fn extracts_actor_bootstrap_field_shape_from_parsed_source() {
    let ast = parse_source("actor DocHub id string { nextSeq: number, pendingOps: Array<string> }")
        .unwrap();
    let declarations = actor_declaration_inputs(&ast);
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name, "DocHub");
    assert_eq!(declarations[0].id_type.name, "string");
    assert_eq!(declarations[0].fields[1].name, "pendingOps");
    assert_eq!(declarations[0].fields[1].ty.name, "Array<string>");
}
