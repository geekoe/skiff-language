use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    CallableRegistryMatch, CallableRegistryMatchError, CallableRegistrySignature, MetadataValue,
    NativeTarget, ValueLifecycleFactResolver, ValueLifecyclePolicyBudget, ValueProvenance,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEffectRegistryIdentity {
    pub registry_id: String,
    pub version: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostEffectRequiredContext {
    None,
    Actor,
    File,
    Time,
    HttpClient,
    HttpResponseStream,
    Websocket,
    Telemetry,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostEffectMetadataShape {
    Null,
    Bool,
    Number,
    String,
    Array {
        items: Box<HostEffectMetadataShape>,
    },
    Object {
        fields: BTreeMap<String, HostEffectMetadataShape>,
    },
}

/// Exact metadata schema. Keys not present in `fields` are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEffectMetadataMatcher {
    pub fields: BTreeMap<String, HostEffectMetadataShape>,
}

impl HostEffectMetadataMatcher {
    pub fn matches(&self, metadata: &BTreeMap<String, MetadataValue>) -> bool {
        metadata.len() == self.fields.len()
            && self
                .fields
                .iter()
                .all(|(name, shape)| metadata.get(name).is_some_and(|value| shape.matches(value)))
    }
}

impl HostEffectMetadataShape {
    fn matches(&self, value: &MetadataValue) -> bool {
        match (self, value) {
            (Self::Null, MetadataValue::Null)
            | (Self::Bool, MetadataValue::Bool(_))
            | (Self::Number, MetadataValue::Number(_))
            | (Self::String, MetadataValue::String(_)) => true,
            (Self::Array { items }, MetadataValue::Array(values)) => {
                values.iter().all(|value| items.matches(value))
            }
            (Self::Object { fields }, MetadataValue::Object(values)) => {
                values.len() == fields.len()
                    && fields.iter().all(|(name, shape)| {
                        values.get(name).is_some_and(|value| shape.matches(value))
                    })
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostEffectReceiverSemantics {
    None,
    ExplicitArgument {
        parameter_ordinal: u32,
        mutates_receiver: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEffectRegistryEntry {
    pub target: String,
    pub aliases: Vec<String>,
    pub binding_key: String,
    pub abi_version: u32,
    pub required_context: HostEffectRequiredContext,
    pub metadata: HostEffectMetadataMatcher,
    pub receiver: HostEffectReceiverSemantics,
    pub signature: CallableRegistrySignature,
    pub return_provenance: ValueProvenance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostEffectRegistryMatch {
    pub entry: &'static HostEffectRegistryEntry,
    pub signature: CallableRegistryMatch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostEffectRegistryMatchError {
    MissingBindingKey,
    UnknownTarget { target: String },
    BindingKeyMismatch { expected: String, actual: String },
    MetadataMismatch,
    Signature { source: CallableRegistryMatchError },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEffectRegistryBuildError {
    EmptyIdentity,
    EmptyLookupKey { entry: usize },
    NonCanonicalAliases { entry: usize },
    LookupKeyCollision { key: String },
    BindingKeyCollision { binding_key: String },
    ZeroAbiVersion { entry: usize },
    InvalidSignature { entry: usize, message: &'static str },
    InvalidReceiver { entry: usize },
    Fingerprint { message: String },
}

pub(super) fn canonical_target(target: &NativeTarget) -> String {
    if target.namespace.is_empty() {
        target.symbol.clone()
    } else {
        format!("{}.{}", target.namespace, target.symbol)
    }
}

pub(super) fn match_entry<R: ValueLifecycleFactResolver>(
    entry: &'static HostEffectRegistryEntry,
    target: &NativeTarget,
    signature: &crate::bytecode::HostEffectSignature,
    resolver: &mut R,
    budget: &mut ValueLifecyclePolicyBudget,
) -> Result<HostEffectRegistryMatch, HostEffectRegistryMatchError> {
    let binding_key = target
        .binding_key
        .as_deref()
        .ok_or(HostEffectRegistryMatchError::MissingBindingKey)?;
    if binding_key != entry.binding_key {
        return Err(HostEffectRegistryMatchError::BindingKeyMismatch {
            expected: entry.binding_key.clone(),
            actual: binding_key.to_string(),
        });
    }
    if !entry.metadata.matches(&target.metadata) {
        return Err(HostEffectRegistryMatchError::MetadataMismatch);
    }
    let signature =
        crate::match_callable_registry_signature(&entry.signature, signature, resolver, budget)
            .map_err(|source| HostEffectRegistryMatchError::Signature { source })?;
    Ok(HostEffectRegistryMatch { entry, signature })
}
