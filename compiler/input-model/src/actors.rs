use serde::{Deserialize, Serialize};
use skiff_syntax::ast::{ActorDecl, SourceFile, TypeRef};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorDeclarationInput {
    pub name: String,
    pub id_type: TypeRef,
    pub fields: Vec<ActorFieldInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorFieldInput {
    pub name: String,
    pub ty: TypeRef,
}

impl From<&ActorDecl> for ActorDeclarationInput {
    fn from(actor: &ActorDecl) -> Self {
        Self {
            name: actor.name.clone(),
            id_type: actor.id_type.clone(),
            fields: actor
                .fields
                .iter()
                .map(|field| ActorFieldInput {
                    name: field.name.clone(),
                    ty: field.ty.clone(),
                })
                .collect(),
        }
    }
}

pub fn actor_declaration_inputs(ast: &SourceFile) -> Vec<ActorDeclarationInput> {
    ast.actors.iter().map(ActorDeclarationInput::from).collect()
}

#[cfg(test)]
mod tests {
    use skiff_syntax::parser::parse_source;

    use super::*;

    #[test]
    fn extracts_actor_bootstrap_field_shape_from_parsed_source() {
        let ast =
            parse_source("actor DocHub id string { nextSeq: number, pendingOps: Array<string> }")
                .unwrap();
        let declarations = actor_declaration_inputs(&ast);
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].name, "DocHub");
        assert_eq!(declarations[0].id_type.name, "string");
        assert_eq!(declarations[0].fields[1].name, "pendingOps");
        assert_eq!(declarations[0].fields[1].ty.name, "Array<string>");
    }
}
