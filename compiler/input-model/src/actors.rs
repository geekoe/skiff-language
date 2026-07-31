use serde::{Deserialize, Serialize};
use skiff_syntax::ast::{SourceFile, TypeRef};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorDeclarationInput {
    pub name: String,
    pub key_field: String,
    pub fields: Vec<ActorFieldInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<ActorCreateInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorFieldInput {
    pub name: String,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorCreateInput {
    pub params: Vec<ActorCreateParamInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorCreateParamInput {
    pub name: String,
    pub ty: TypeRef,
}

pub fn actor_declaration_inputs(ast: &SourceFile) -> Vec<ActorDeclarationInput> {
    ast.actors
        .iter()
        .map(|actor| {
            let fields = ast
                .types
                .iter()
                .find(|ty| ty.name == actor.name)
                .map(|ty| {
                    ty.fields
                        .iter()
                        .map(|field| ActorFieldInput {
                            name: field.name.clone(),
                            ty: field.ty.clone(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            ActorDeclarationInput {
                name: actor.name.clone(),
                key_field: actor.key_field.clone(),
                fields,
                create: actor.create.as_ref().map(|create| ActorCreateInput {
                    params: create
                        .params
                        .iter()
                        .map(|param| ActorCreateParamInput {
                            name: param.name.clone(),
                            ty: param.ty.clone(),
                        })
                        .collect(),
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
