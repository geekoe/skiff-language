use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{FunctionTypeParamIr, TypeRefIr};

pub const ACTOR_RUNTIME_ABI_VERSION_V1: &str = "skiff-actor-runtime-abi-v1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActorAbiIdentity(String);

impl ActorAbiIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorDeclarationIr {
    pub actor_abi_identity: ActorAbiIdentity,
    pub abi: ActorAbiInput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "ActorAbiInputWire"
)]
pub struct ActorAbiInput {
    pub actor_name: String,
    pub actor_id_type: TypeRefIr,
    pub fields: Vec<ActorFieldIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_methods: Vec<ActorPublicMethodIr>,
    pub actor_runtime_abi_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActorAbiInputWire {
    actor_name: String,
    actor_id_type: TypeRefIr,
    fields: Vec<ActorFieldIr>,
    #[serde(default)]
    public_methods: Vec<ActorPublicMethodIr>,
    actor_runtime_abi_version: String,
}

impl TryFrom<ActorAbiInputWire> for ActorAbiInput {
    type Error = String;

    fn try_from(wire: ActorAbiInputWire) -> Result<Self, Self::Error> {
        if wire.actor_name.trim().is_empty() {
            return Err("actorName must be non-empty".to_string());
        }
        if wire.actor_runtime_abi_version != ACTOR_RUNTIME_ABI_VERSION_V1 {
            return Err(format!(
                "unsupported actorRuntimeAbiVersion {}",
                wire.actor_runtime_abi_version
            ));
        }
        reject_actor_ref(&wire.actor_id_type)?;
        let mut field_names = BTreeSet::new();
        for field in &wire.fields {
            if !field_names.insert(field.name.as_str()) {
                return Err(format!("duplicate actor field {}", field.name));
            }
            reject_actor_ref(&field.ty)?;
        }
        let mut method_names = BTreeSet::new();
        for method in &wire.public_methods {
            if !method_names.insert(method.name.as_str()) {
                return Err(format!("duplicate actor public method {}", method.name));
            }
            for parameter in &method.parameters {
                reject_actor_ref(&parameter.ty)?;
            }
            reject_actor_ref(&method.return_type)?;
        }
        Ok(Self {
            actor_name: wire.actor_name,
            actor_id_type: wire.actor_id_type,
            fields: wire.fields,
            public_methods: wire.public_methods,
            actor_runtime_abi_version: wire.actor_runtime_abi_version,
        })
    }
}

fn reject_actor_ref(ty: &TypeRefIr) -> Result<(), String> {
    match ty {
        TypeRefIr::Builtin { name, args } => {
            if matches!(
                name.as_str(),
                "ActorRef" | "actor.ActorRef" | "std.actor.ActorRef"
            ) {
                return Err("legacy ActorRef is not a public artifact type".to_string());
            }
            args.iter().try_for_each(reject_actor_ref)
        }
        TypeRefIr::Record { fields } => fields.values().try_for_each(reject_actor_ref),
        TypeRefIr::Union { items } => items.iter().try_for_each(reject_actor_ref),
        TypeRefIr::Nullable { inner } => reject_actor_ref(inner),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .try_for_each(|parameter| reject_actor_ref(&parameter.ty))?;
            reject_actor_ref(return_type)
        }
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .try_for_each(reject_actor_ref),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn actor_declaration_wire_preserves_field_layout_and_encoding() {
        let wire = json!({
            "actorName": "DocHub",
            "actorIdType": { "kind": "builtin", "name": "string" },
            "fields": [
                {
                    "name": "nextSeq",
                    "ty": { "kind": "builtin", "name": "number" },
                    "encoding": "canonicalValueV1"
                }
            ],
            "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
        });
        let abi: ActorAbiInput = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(abi.fields[0].name, "nextSeq");
        assert_eq!(serde_json::to_value(abi).unwrap(), wire);
    }

    #[test]
    fn actor_abi_wire_rejects_actor_ref_and_noncanonical_shape() {
        let actor_ref = json!({
            "actorName": "DocHub",
            "actorIdType": {
                "kind": "builtin",
                "name": "ActorRef",
                "args": [{ "kind": "builtin", "name": "string" }]
            },
            "fields": [],
            "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
        });
        assert!(serde_json::from_value::<ActorAbiInput>(actor_ref)
            .unwrap_err()
            .to_string()
            .contains("legacy ActorRef"));

        let duplicate_fields = json!({
            "actorName": "DocHub",
            "actorIdType": { "kind": "builtin", "name": "string" },
            "fields": [
                {
                    "name": "value",
                    "ty": { "kind": "builtin", "name": "string" },
                    "encoding": "canonicalValueV1"
                },
                {
                    "name": "value",
                    "ty": { "kind": "builtin", "name": "number" },
                    "encoding": "canonicalValueV1"
                }
            ],
            "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
        });
        assert!(serde_json::from_value::<ActorAbiInput>(duplicate_fields)
            .unwrap_err()
            .to_string()
            .contains("duplicate actor field"));
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorFieldIr {
    pub name: String,
    pub ty: TypeRefIr,
    pub encoding: ActorFieldEncodingIr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorFieldEncodingIr {
    CanonicalValueV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorPublicMethodIr {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
    pub may_suspend: bool,
}
