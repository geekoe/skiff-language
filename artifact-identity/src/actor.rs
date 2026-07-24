use serde::Serialize;
use skiff_artifact_model::{ActorAbiIdentity, ActorAbiInput};

use crate::{
    framing::{canonical_ir_bytes, framed_identity, sha256_hex},
    ArtifactIdentityError, Result, ACTOR_ABI_IDENTITY_PREFIX, ACTOR_ABI_IDENTITY_SCHEMA_MARKER,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActorAbiIdentityPreimage<'a> {
    schema: &'static str,
    abi: &'a ActorAbiInput,
}

pub fn actor_abi_identity(abi: &ActorAbiInput) -> Result<ActorAbiIdentity> {
    let bytes = canonical_ir_bytes(
        &ActorAbiIdentityPreimage {
            schema: ACTOR_ABI_IDENTITY_SCHEMA_MARKER,
            abi,
        },
        ArtifactIdentityError::SerializeActorAbiIdentity,
    )?;
    Ok(ActorAbiIdentity::new(framed_identity(
        ACTOR_ABI_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        ActorFieldEncodingIr, ActorFieldIr, ActorPublicMethodIr, TypeRefIr,
        ACTOR_RUNTIME_ABI_VERSION_V1,
    };

    use super::*;

    fn abi() -> ActorAbiInput {
        ActorAbiInput {
            actor_name: "DocHub".to_string(),
            actor_id_type: TypeRefIr::builtin("string"),
            fields: vec![ActorFieldIr {
                name: "nextSeq".to_string(),
                ty: TypeRefIr::builtin("number"),
                encoding: ActorFieldEncodingIr::CanonicalValueV1,
            }],
            public_methods: Vec::new(),
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        }
    }

    #[test]
    fn actor_abi_identity_covers_id_fields_and_runtime_version() {
        let base = actor_abi_identity(&abi()).unwrap();
        let mut changed_id = abi();
        changed_id.actor_id_type = TypeRefIr::builtin("integer");
        assert_ne!(base, actor_abi_identity(&changed_id).unwrap());

        let mut changed_field = abi();
        changed_field.fields[0].ty = TypeRefIr::builtin("integer");
        assert_ne!(base, actor_abi_identity(&changed_field).unwrap());

        let mut changed_runtime = abi();
        changed_runtime.actor_runtime_abi_version = "skiff-actor-runtime-abi-v2".to_string();
        assert_ne!(base, actor_abi_identity(&changed_runtime).unwrap());

        let mut changed_methods = abi();
        changed_methods.public_methods.push(ActorPublicMethodIr {
            name: "append".to_string(),
            parameters: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            may_suspend: false,
        });
        assert_ne!(base, actor_abi_identity(&changed_methods).unwrap());

        assert!(base.as_str().starts_with(ACTOR_ABI_IDENTITY_PREFIX));
    }
}
