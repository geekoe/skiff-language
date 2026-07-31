use skiff_syntax::parser::parse_source;

use super::*;

#[test]
fn extracts_actor_bootstrap_field_shape_from_parsed_source() {
    let ast = parse_source(
        "type DocHub { id: string, nextSeq: number, pendingOps: Array<string> }\n\
         actor DocHub { key(id) create(initialNextSeq: number) }",
    )
    .unwrap();
    let declarations = actor_declaration_inputs(&ast);
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name, "DocHub");
    assert_eq!(declarations[0].key_field, "id");
    assert_eq!(declarations[0].fields[0].name, "id");
    assert_eq!(declarations[0].fields[0].ty.name, "string");
    assert_eq!(declarations[0].fields[2].name, "pendingOps");
    assert_eq!(declarations[0].fields[2].ty.name, "Array<string>");
    let create = declarations[0].create.as_ref().expect("create input");
    assert_eq!(create.params[0].name, "initialNextSeq");
    assert_eq!(create.params[0].ty.name, "number");
}
