use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

use serde::Serialize;
use sha2::Digest;

use crate::{
    callable_signature_from_native, is_runtime_receiver_native_binding_key,
    native_callable_semantics, NativeTarget, ValueLifecycleFactResolver,
    ValueLifecyclePolicyBudget, STD_NATIVE_SIGNATURES,
};

use super::contract::*;

pub const HOST_EFFECT_REGISTRY_ID: &str = "skiff-host-effect-registry";
pub const HOST_EFFECT_REGISTRY_VERSION: &str = "skiff-host-effect-registry-v4";
pub const HOST_EFFECT_REGISTRY_FINGERPRINT: &str =
    "50a4a5fb71b5cd47eac78942be6db46f44544d00529d495e1587319125809c97";

#[derive(Debug)]
pub struct HostEffectRegistry {
    identity: HostEffectRegistryIdentity,
    entries: Vec<HostEffectRegistryEntry>,
}

impl HostEffectRegistry {
    fn built_in() -> Self {
        let entries = STD_NATIVE_SIGNATURES
            .iter()
            .filter(|signature| !is_intrinsic(signature.binding_key))
            .filter_map(|signature| {
                let semantics = native_callable_semantics(signature.binding_key)?;
                let required_context =
                    required_context(signature.binding_key).unwrap_or_else(|| {
                        panic!(
                            "audited host binding lacks required-context authority: {}",
                            signature.binding_key
                        )
                    });
                Some(HostEffectRegistryEntry {
                    target: signature.target.to_string(),
                    aliases: signature.aliases.iter().map(ToString::to_string).collect(),
                    binding_key: signature.binding_key.to_string(),
                    abi_version: 1,
                    executor_identity: executor_identity(signature.binding_key),
                    required_context,
                    metadata: HostEffectMetadataMatcher {
                        fields: BTreeMap::new(),
                    },
                    receiver: HostEffectReceiverSemantics::None,
                    signature: callable_signature_from_native(signature, semantics.effects.clone()),
                    return_provenance: semantics.return_provenance.clone(),
                })
            })
            .collect::<Vec<_>>();
        let registry = Self::new(
            HOST_EFFECT_REGISTRY_ID,
            HOST_EFFECT_REGISTRY_VERSION,
            entries,
        )
        .expect("built-in host effect registry is valid");
        assert_eq!(
            registry.identity.fingerprint, HOST_EFFECT_REGISTRY_FINGERPRINT,
            "host effect registry changed without a version bump"
        );
        registry
    }

    pub(crate) fn new(
        registry_id: impl Into<String>,
        version: impl Into<String>,
        mut entries: Vec<HostEffectRegistryEntry>,
    ) -> Result<Self, HostEffectRegistryBuildError> {
        let registry_id = registry_id.into();
        let version = version.into();
        if registry_id.trim().is_empty() || version.trim().is_empty() {
            return Err(HostEffectRegistryBuildError::EmptyIdentity);
        }
        entries.sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then_with(|| left.binding_key.cmp(&right.binding_key))
        });
        validate_entries(&entries)?;
        let fingerprint = fingerprint(&registry_id, &version, &entries)?;
        Ok(Self {
            identity: HostEffectRegistryIdentity {
                registry_id,
                version,
                fingerprint,
            },
            entries,
        })
    }

    pub fn identity(&self) -> &HostEffectRegistryIdentity {
        &self.identity
    }

    pub fn entries(&self) -> &[HostEffectRegistryEntry] {
        &self.entries
    }

    pub fn match_reference<R: ValueLifecycleFactResolver>(
        &'static self,
        target: &NativeTarget,
        signature: &crate::bytecode::HostEffectSignature,
        resolver: &mut R,
        budget: &mut ValueLifecyclePolicyBudget,
    ) -> Result<HostEffectRegistryMatch, HostEffectRegistryMatchError> {
        let canonical = canonical_target(target);
        let entry = self
            .entries
            .iter()
            .find(|entry| {
                entry.target == canonical || entry.aliases.iter().any(|alias| alias == &canonical)
            })
            .ok_or(HostEffectRegistryMatchError::UnknownTarget { target: canonical })?;
        match_entry(entry, target, signature, resolver, budget)
    }
}

fn is_intrinsic(binding_key: &str) -> bool {
    binding_key == "core.array.empty"
        || binding_key == "core.map.empty"
        || binding_key == "core.bytes.fromUtf8"
        || is_runtime_receiver_native_binding_key(binding_key)
}

fn executor_identity(binding_key: &str) -> Option<HostEffectExecutorIdentity> {
    Some(match binding_key {
        "std.time.sleep" => HostEffectExecutorIdentity::Sleep,
        "std.http.client.request" => HostEffectExecutorIdentity::HttpClientRequest,
        "std.http.client.stream" => HostEffectExecutorIdentity::HttpClientStream,
        _ => return None,
    })
}

fn validate_entries(
    entries: &[HostEffectRegistryEntry],
) -> Result<(), HostEffectRegistryBuildError> {
    let mut lookup_keys = BTreeSet::new();
    let mut binding_keys = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if !is_lookup_key_valid(&entry.target) || !is_lookup_key_valid(&entry.binding_key) {
            return Err(HostEffectRegistryBuildError::EmptyLookupKey { entry: index });
        }
        if entry.abi_version == 0 {
            return Err(HostEffectRegistryBuildError::ZeroAbiVersion { entry: index });
        }
        validate_signature(index, &entry.signature)?;
        if let HostEffectReceiverSemantics::ExplicitArgument {
            parameter_ordinal, ..
        } = entry.receiver
        {
            let ordinal = parameter_ordinal as usize;
            if ordinal >= entry.signature.parameter_types.len()
                || entry.signature.parameter_modes[ordinal] != crate::ParamModeIr::Value
            {
                return Err(HostEffectRegistryBuildError::InvalidReceiver { entry: index });
            }
        }
        if !binding_keys.insert(entry.binding_key.clone()) {
            return Err(HostEffectRegistryBuildError::BindingKeyCollision {
                binding_key: entry.binding_key.clone(),
            });
        }
        if !lookup_keys.insert(entry.target.clone()) {
            return Err(HostEffectRegistryBuildError::LookupKeyCollision {
                key: entry.target.clone(),
            });
        }
        if !entry.aliases.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(HostEffectRegistryBuildError::NonCanonicalAliases { entry: index });
        }
        for alias in &entry.aliases {
            if !is_lookup_key_valid(alias) || alias == &entry.target {
                return Err(HostEffectRegistryBuildError::EmptyLookupKey { entry: index });
            }
            if !lookup_keys.insert(alias.clone()) {
                return Err(HostEffectRegistryBuildError::LookupKeyCollision {
                    key: alias.clone(),
                });
            }
        }
    }
    Ok(())
}

fn is_lookup_key_valid(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

fn validate_signature(
    entry: usize,
    signature: &crate::CallableRegistrySignature,
) -> Result<(), HostEffectRegistryBuildError> {
    crate::callable_registry::validate_callable_registry_signature(signature)
        .map_err(|message| HostEffectRegistryBuildError::InvalidSignature { entry, message })
}

fn fingerprint(
    registry_id: &str,
    version: &str,
    entries: &[HostEffectRegistryEntry],
) -> Result<String, HostEffectRegistryBuildError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Projection<'a> {
        registry_id: &'a str,
        version: &'a str,
        entries: &'a [HostEffectRegistryEntry],
    }
    let bytes = skiff_canonical_json::canonical_json_bytes(&Projection {
        registry_id,
        version,
        entries,
    })
    .map_err(|error| HostEffectRegistryBuildError::Fingerprint {
        message: error.to_string(),
    })?;
    Ok(hex::encode(sha2::Sha256::digest(bytes)))
}

fn required_context(binding_key: &str) -> Option<HostEffectRequiredContext> {
    use HostEffectRequiredContext as Context;
    Some(match binding_key {
        "std.config.require" | "std.config.optional" | "std.config.has" => Context::Config,
        "std.db.operation" => Context::Db,
        "std.actor.get" => Context::Actor,
        "core.date.now" | "std.time.sleep" => Context::Time,
        "std.http.client.request" | "std.http.client.stream" | "std.http.client.sse" => {
            Context::HttpClient
        }
        "std.http.stream.emitResponse" => Context::HttpResponseStream,
        "std.file.create"
        | "std.file.createText"
        | "std.file.read"
        | "std.file.readText"
        | "std.file.info"
        | "std.file.delete"
        | "std.file.createFromStream" => Context::File,
        "std.resource.bytes"
        | "std.resource.text"
        | "std.resource.json"
        | "std.resource.info"
        | "std.resource.exists" => Context::Resource,
        "std.telemetry.emit" => Context::Telemetry,
        "std.websocket.sendTextToConnection"
        | "std.websocket.sendBinaryToConnection"
        | "std.websocket.sendTextToBusinessIdentity"
        | "std.websocket.sendBinaryToBusinessIdentity"
        | "std.websocket.requestJsonToConnection" => Context::Websocket,
        "core.date.fromEpochMilliseconds"
        | "core.date.parse"
        | "core.date.requireParse"
        | "core.date.toEpochMilliseconds"
        | "core.date.toISOString"
        | "core.date.addMilliseconds"
        | "core.date.diffMilliseconds"
        | "core.date.compare"
        | "core.date.isBefore"
        | "core.date.isAfter"
        | "core.duration.milliseconds"
        | "core.duration.seconds"
        | "core.duration.toMilliseconds"
        | "core.number.parse"
        | "core.number.isInteger"
        | "core.number.isSafeInteger"
        | "core.number.assertSafeInteger"
        | "core.bytes.fromBase64"
        | "core.bytes.fromHex"
        | "core.bytes.fromUtf8"
        | "core.bytes.concat"
        | "std.json.encode"
        | "std.json.decode"
        | "std.json.merge"
        | "std.json.get"
        | "std.json.getString"
        | "std.json.getNumber"
        | "std.json.getBool"
        | "std.json.getArray"
        | "std.string.join"
        | "std.string.split"
        | "std.string.isAsciiDigits"
        | "std.string.truncateUtf8Bytes"
        | "std.string.encodeQueryComponent"
        | "std.string.encodePath"
        | "std.crypto.hmacSha1Base64"
        | "std.crypto.sha256"
        | "std.crypto.randomToken"
        | "std.crypto.uuid"
        | "std.crypto.uuidSimple"
        | "std.http.request.header"
        | "std.http.request.headers"
        | "std.http.request.query"
        | "std.http.request.cookie"
        | "std.http.request.decodeJson"
        | "std.http.request.requireMethod"
        | "std.http.response.json"
        | "std.http.response.jsonWithHeaders"
        | "std.http.response.error"
        | "std.http.response.noContent"
        | "std.http.response.methodNotAllowed"
        | "std.http.headers.forwardable"
        | "std.http.headers.sse"
        | "std.http.stream.start"
        | "std.http.stream.chunk"
        | "std.http.stream.end"
        | "std.task.status"
        | "std.task.cancel" => Context::None,
        _ => return None,
    })
}

pub static HOST_EFFECT_REGISTRY: LazyLock<HostEffectRegistry> =
    LazyLock::new(HostEffectRegistry::built_in);

pub fn host_effect_registry() -> &'static HostEffectRegistry {
    &HOST_EFFECT_REGISTRY
}

pub fn host_effect_registry_identity() -> &'static HostEffectRegistryIdentity {
    HOST_EFFECT_REGISTRY.identity()
}
