use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActorMethodIdentity(String);

impl ActorMethodIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActorImplementationIdentity(String);

impl ActorImplementationIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "ActorDeclarationIrWire"
)]
pub struct ActorDeclarationIr {
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub abi: ActorAbiInput,
    pub method_implementations: BTreeMap<ActorMethodIdentity, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_implementation: Option<ActorCreateImplementationIr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActorDeclarationIrWire {
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: ActorImplementationIdentity,
    abi: ActorAbiInput,
    method_implementations: BTreeMap<ActorMethodIdentity, u32>,
    #[serde(default)]
    create_implementation: Option<ActorCreateImplementationIr>,
}

impl TryFrom<ActorDeclarationIrWire> for ActorDeclarationIr {
    type Error = String;

    fn try_from(wire: ActorDeclarationIrWire) -> Result<Self, Self::Error> {
        let public = wire
            .abi
            .public_methods
            .iter()
            .map(|method| method.method_identity.clone())
            .collect::<BTreeSet<_>>();
        let implementations = wire
            .method_implementations
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if public != implementations {
            return Err(
                "actor methodImplementations must match publicMethods methodIdentity values"
                    .to_string(),
            );
        }
        if wire.abi.create.is_none() && wire.create_implementation.is_some() {
            return Err(
                "actor createImplementation requires a declared create signature".to_string(),
            );
        }
        if wire.abi.create.is_some() && wire.create_implementation.is_none() {
            return Err(
                "actor declared create signature requires a create implementation".to_string(),
            );
        }
        if let Some(create) = wire.create_implementation.as_ref() {
            if public.contains(&create.identity) {
                return Err(
                    "actor create method identity must not be a public method identity".to_string(),
                );
            }
        }
        Ok(Self {
            actor_abi_identity: wire.actor_abi_identity,
            actor_implementation_identity: wire.actor_implementation_identity,
            abi: wire.abi,
            method_implementations: wire.method_implementations,
            create_implementation: wire.create_implementation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorCreateImplementationIr {
    pub identity: ActorMethodIdentity,
    pub executable_index: u32,
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
    pub key_field: String,
    pub fields: Vec<ActorFieldIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<ActorCreateSignatureIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_methods: Vec<ActorPublicMethodIr>,
    pub actor_runtime_abi_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActorAbiInputWire {
    actor_name: String,
    actor_id_type: TypeRefIr,
    key_field: String,
    fields: Vec<ActorFieldIr>,
    #[serde(default)]
    create: Option<ActorCreateSignatureIr>,
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
        if wire.key_field.trim().is_empty() {
            return Err("keyField must be non-empty".to_string());
        }
        let key_type = wire
            .fields
            .iter()
            .find(|field| field.name == wire.key_field)
            .map(|field| &field.ty)
            .ok_or_else(|| format!("actor key field {} is absent from fields", wire.key_field))?;
        if key_type != &wire.actor_id_type {
            return Err("actorIdType must exactly match the key field type".to_string());
        }
        let mut field_names = BTreeSet::new();
        for field in &wire.fields {
            if !field_names.insert(field.name.as_str()) {
                return Err(format!("duplicate actor field {}", field.name));
            }
            reject_actor_ref(&field.ty)?;
        }
        if let Some(create) = wire.create.as_ref() {
            for parameter in &create.parameters {
                reject_actor_ref(&parameter.ty)?;
            }
        }
        let mut method_names = BTreeSet::new();
        let mut method_identities = BTreeSet::new();
        for method in &wire.public_methods {
            if method.name == "create" {
                return Err("actor public method must not be named create".to_string());
            }
            if !method_names.insert(method.name.as_str()) {
                return Err(format!("duplicate actor public method {}", method.name));
            }
            if !method_identities.insert(method.method_identity.as_str()) {
                return Err(format!(
                    "duplicate actor public method identity {}",
                    method.method_identity.as_str()
                ));
            }
            for parameter in &method.parameters {
                reject_actor_ref(&parameter.ty)?;
            }
            reject_actor_ref(&method.return_type)?;
        }
        Ok(Self {
            actor_name: wire.actor_name,
            actor_id_type: wire.actor_id_type,
            key_field: wire.key_field,
            fields: wire.fields,
            create: wire.create,
            public_methods: wire.public_methods,
            actor_runtime_abi_version: wire.actor_runtime_abi_version,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorCreateSignatureIr {
    pub parameters: Vec<FunctionTypeParamIr>,
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
        TypeRefIr::AppliedNominal { base, arguments } => {
            if arguments.is_empty() {
                return Err("appliedNominal arguments must be non-empty".to_string());
            }
            if matches!(base, crate::NominalTypeRefBaseIr::PackageSchema { .. }) {
                return Err(
                    "applied PackageSchema is not admitted in this artifact generation".to_string(),
                );
            }
            arguments.iter().try_for_each(reject_actor_ref)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests;

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
    pub method_identity: ActorMethodIdentity,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
    pub may_suspend: bool,
}
