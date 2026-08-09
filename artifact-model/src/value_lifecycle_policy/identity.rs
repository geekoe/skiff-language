use std::sync::LazyLock;

use sha2::Digest;

use super::contract::ValueLifecyclePolicyIdentity;

pub const VALUE_LIFECYCLE_POLICY_VERSION: &str = "skiff-bytecode-value-lifecycle-policy-v1";
pub const VALUE_LIFECYCLE_POLICY_FINGERPRINT: &str =
    "2d478cdc3e2226f41b98dfa6cfbed5712b1587987e49c278f887e45486b3ec8a";

static VALUE_LIFECYCLE_POLICY_IDENTITY: LazyLock<ValueLifecyclePolicyIdentity> =
    LazyLock::new(|| {
        let fingerprint = fingerprint_projection(&policy_projection());
        assert_eq!(
            fingerprint, VALUE_LIFECYCLE_POLICY_FINGERPRINT,
            "value lifecycle policy changed without a version bump"
        );
        ValueLifecyclePolicyIdentity {
            version: VALUE_LIFECYCLE_POLICY_VERSION.to_string(),
            fingerprint,
        }
    });

pub fn value_lifecycle_policy_identity() -> &'static ValueLifecyclePolicyIdentity {
    &VALUE_LIFECYCLE_POLICY_IDENTITY
}

pub(super) fn policy_projection() -> serde_json::Value {
    serde_json::json!({
        "adapters": "bindingKeyExactRegistryRoleAndAbi",
        "alias": "transparentAfterOwnerNormalization",
        "anyInterface": "authorityExactTargetAndArity+ordinarySnapshotArguments->snapshotShare(snapshotRelease)",
        "budget": "sharedNodesBytesDepth",
        "cycles": "exactInstantiatedNominalKeyVisitingReject+successfulResolutionMemo",
        "descriptors": {
            "callbackInterface": "bareReject",
            "discriminatedUnion": "ordinarySnapshotBranches->snapshotShare(snapshotRelease)",
            "enumeration": "frozenSnapshotRoot(snapshotRelease)",
            "record": "ordinarySnapshotFields->snapshotShare(snapshotRelease)",
            "representation": "exactChild+requireOrdinarySnapshot",
            "structuralUnion": "ordinarySnapshotVariants->snapshotShare(snapshotRelease)",
        },
        "genericSubstitution": "declarationOrdinalScoped",
        "literal": "exactDomainBuiltinLifecycle",
        "nativeRegistry": {
            "fingerprint": crate::NATIVE_VALUE_LIFECYCLE_REGISTRY_FINGERPRINT,
            "registryId": crate::NATIVE_VALUE_LIFECYCLE_REGISTRY_ID,
            "version": crate::NATIVE_VALUE_LIFECYCLE_REGISTRY_VERSION,
        },
        "nullable": "ordinarySnapshotChild->snapshotShare(snapshotRelease)",
        "ownerNormalization": "resolvedPackageId+exactAbi;rejectLocalPublicationDependencyServiceDb",
        "packageSchema": "exactOwnerStableKeyTypeId+descriptorClosure",
        "planVerification": "FromTypeExactNormalizedType+concreteExactRecompute",
        "recursiveShape": "reject",
        "version": VALUE_LIFECYCLE_POLICY_VERSION,
    })
}

pub(super) fn fingerprint_projection(projection: &serde_json::Value) -> String {
    let bytes = skiff_canonical_json::canonical_json_bytes(projection)
        .expect("value lifecycle policy projection is canonicalizable");
    hex::encode(sha2::Sha256::digest(bytes))
}
