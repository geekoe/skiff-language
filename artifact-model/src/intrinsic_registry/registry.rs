use std::sync::LazyLock;

use serde::Serialize;
use sha2::Digest;

use crate::{
    builtin_receiver_callable_semantics, callable_signature_from_native, native_callable_semantics,
    native_signature_for_receiver_op, BuiltinReceiverPublicReturnType, BytecodeIntrinsicRef,
    ValueLifecycleFactResolver, ValueLifecyclePolicyBudget, STD_NATIVE_SIGNATURES,
    SUPPORTED_RECEIVER_BUILTIN_OPS,
};

use super::contract::*;

pub const INTRINSIC_REGISTRY_ID: &str = "skiff-intrinsic-registry";
pub const INTRINSIC_REGISTRY_VERSION: &str = "skiff-intrinsic-registry-v1";
pub const INTRINSIC_REGISTRY_FINGERPRINT: &str =
    "fc6c7ab282d3b5d3cad79a84fec84d71d0749d6d75cbd3dbb0bb7b96e7cd7c61";
pub const UNSUPPORTED_INTRINSIC_RECEIVER_KEYS: &[&str] = &[
    "receiver:Array.clone@1",
    "receiver:Array.length@1",
    "receiver:Array.pop@1",
    "receiver:Array.push@1",
    "receiver:Array.set@1",
    "receiver:Date.isAfter@1",
    "receiver:Date.toISOString@1",
    "receiver:JsonObject.clone@1",
    "receiver:JsonObject.delete@1",
    "receiver:JsonObject.get@1",
    "receiver:JsonObject.has@1",
    "receiver:JsonObject.length@1",
    "receiver:JsonObject.set@1",
    "receiver:Map.clone@1",
    "receiver:Map.delete@1",
    "receiver:Map.get@1",
    "receiver:Map.has@1",
    "receiver:Map.keys@1",
    "receiver:Map.length@1",
    "receiver:Map.set@1",
    "receiver:bytes.length@1",
    "receiver:bytes.toBase64@1",
    "receiver:bytes.toHex@1",
    "receiver:bytes.toUtf8String@1",
    "receiver:number.ceil@1",
    "receiver:number.floor@1",
    "receiver:number.round@1",
    "receiver:string.concat@1",
    "receiver:string.contains@1",
    "receiver:string.endsWith@1",
    "receiver:string.length@1",
    "receiver:string.lowercase@1",
    "receiver:string.replaceAll@1",
    "receiver:string.startsWith@1",
];

#[derive(Debug)]
pub struct IntrinsicRegistry {
    identity: IntrinsicRegistryIdentity,
    entries: Vec<IntrinsicRegistryEntry>,
}

impl IntrinsicRegistry {
    fn built_in() -> Self {
        let mut entries = static_entries();
        entries.extend(receiver_entries());
        entries.sort_by(|left, right| target_key(&left.target).cmp(&target_key(&right.target)));
        for entry in &entries {
            crate::callable_registry::validate_callable_registry_signature(&entry.signature)
                .expect("built-in intrinsic signature is valid");
        }
        for pair in entries.windows(2) {
            assert_ne!(
                target_key(&pair[0].target),
                target_key(&pair[1].target),
                "intrinsic registry targets are unique"
            );
        }
        let fingerprint = fingerprint(&entries);
        assert_eq!(
            fingerprint, INTRINSIC_REGISTRY_FINGERPRINT,
            "intrinsic registry changed without a version bump"
        );
        Self {
            identity: IntrinsicRegistryIdentity {
                registry_id: INTRINSIC_REGISTRY_ID.to_string(),
                version: INTRINSIC_REGISTRY_VERSION.to_string(),
                fingerprint,
            },
            entries,
        }
    }

    pub fn identity(&self) -> &IntrinsicRegistryIdentity {
        &self.identity
    }

    pub fn entries(&self) -> &[IntrinsicRegistryEntry] {
        &self.entries
    }

    pub fn match_reference<R: ValueLifecycleFactResolver>(
        &'static self,
        reference: &crate::bytecode::IntrinsicReference,
        resolver: &mut R,
        budget: &mut ValueLifecyclePolicyBudget,
    ) -> Result<IntrinsicRegistryMatch, IntrinsicRegistryMatchError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.target == reference.target)
            .ok_or(IntrinsicRegistryMatchError::UnknownTarget)?;
        match_entry(entry, reference, resolver, budget)
    }
}

fn static_entries() -> Vec<IntrinsicRegistryEntry> {
    [
        ("Array.empty", "core.array.empty", "core.array.empty"),
        ("Map.empty", "core.map.empty", "core.map.empty"),
    ]
    .into_iter()
    .map(|(target, binding_key, canonical_key)| {
        let signature = STD_NATIVE_SIGNATURES
            .iter()
            .find(|signature| signature.target == target && signature.binding_key == binding_key)
            .expect("static intrinsic has an exact native signature");
        let semantics = native_callable_semantics(binding_key)
            .expect("static intrinsic has audited callable semantics");
        assert!(!semantics.effects.may_pending());
        IntrinsicRegistryEntry {
            target: BytecodeIntrinsicRef::Static {
                canonical_key: canonical_key.to_string(),
                signature_version: 1,
            },
            introduced_capability_version: 1,
            receiver: IntrinsicReceiverSemantics::Static,
            signature: callable_signature_from_native(signature, semantics.effects.clone()),
            return_provenance: semantics.return_provenance.clone(),
        }
    })
    .collect()
}

fn receiver_entries() -> Vec<IntrinsicRegistryEntry> {
    let mut entries = Vec::new();
    for spec in SUPPORTED_RECEIVER_BUILTIN_OPS {
        match (
            native_signature_for_receiver_op(spec.op),
            builtin_receiver_callable_semantics(spec.op),
        ) {
            (Some(signature), Some(semantics)) if !semantics.effects.may_pending() => {
                entries.push(IntrinsicRegistryEntry {
                    target: BytecodeIntrinsicRef::Receiver { op: spec.op },
                    introduced_capability_version: spec.introduced_capability_version,
                    receiver: IntrinsicReceiverSemantics::Receiver {
                        parameter_ordinal: 0,
                        mutates_receiver: spec.mutates_receiver,
                        throws: spec.throws,
                        public_return_type: public_return_type(spec.public_return_type),
                    },
                    signature: callable_signature_from_native(signature, semantics.effects.clone()),
                    return_provenance: semantics.return_provenance.clone(),
                });
            }
            _ => assert!(
                UNSUPPORTED_INTRINSIC_RECEIVER_KEYS
                    .binary_search(&spec.op.canonical_key)
                    .is_ok(),
                "supported receiver op lacks registry facts without an explicit unsupported row: {}",
                spec.op.canonical_key
            ),
        }
    }
    assert_eq!(
        entries.len() + UNSUPPORTED_INTRINSIC_RECEIVER_KEYS.len(),
        SUPPORTED_RECEIVER_BUILTIN_OPS.len(),
        "receiver intrinsic entries and explicit unsupported rows exact-cover supported ops"
    );
    entries
}

fn public_return_type(value: BuiltinReceiverPublicReturnType) -> IntrinsicPublicReturnType {
    match value {
        BuiltinReceiverPublicReturnType::Fixed(builtin) => IntrinsicPublicReturnType::Fixed {
            builtin: builtin.to_string(),
        },
        BuiltinReceiverPublicReturnType::Receiver => IntrinsicPublicReturnType::Receiver,
        BuiltinReceiverPublicReturnType::ArrayItem => IntrinsicPublicReturnType::ArrayItem,
        BuiltinReceiverPublicReturnType::MapValue => IntrinsicPublicReturnType::MapValue,
        BuiltinReceiverPublicReturnType::MapKeyArray => IntrinsicPublicReturnType::MapKeyArray,
    }
}

fn target_key(target: &BytecodeIntrinsicRef) -> String {
    match target {
        BytecodeIntrinsicRef::Static {
            canonical_key,
            signature_version,
        } => format!("static:{canonical_key}@{signature_version}"),
        BytecodeIntrinsicRef::Receiver { op } => format!("receiver:{}", op.canonical_key),
    }
}

fn fingerprint(entries: &[IntrinsicRegistryEntry]) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Projection<'a> {
        registry_id: &'a str,
        version: &'a str,
        entries: &'a [IntrinsicRegistryEntry],
        unsupported_receiver_keys: &'a [&'a str],
    }
    let bytes = skiff_canonical_json::canonical_json_bytes(&Projection {
        registry_id: INTRINSIC_REGISTRY_ID,
        version: INTRINSIC_REGISTRY_VERSION,
        entries,
        unsupported_receiver_keys: UNSUPPORTED_INTRINSIC_RECEIVER_KEYS,
    })
    .expect("intrinsic registry projection is canonicalizable");
    hex::encode(sha2::Sha256::digest(bytes))
}

pub static INTRINSIC_REGISTRY: LazyLock<IntrinsicRegistry> =
    LazyLock::new(IntrinsicRegistry::built_in);

pub fn intrinsic_registry() -> &'static IntrinsicRegistry {
    &INTRINSIC_REGISTRY
}

pub fn intrinsic_registry_identity() -> &'static IntrinsicRegistryIdentity {
    INTRINSIC_REGISTRY.identity()
}
