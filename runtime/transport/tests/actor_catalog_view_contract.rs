//! A0 actor-routing projection corpus + `ActorMethodCatalogView` reference
//! model for C-actor (`doc/implementation/router-rust-migration-c-actor-contract.md`)
//! and C-model-actor (`doc/implementation/router-rust-migration-c-model-actor-contract.md`).
//!
//! The projection schema is frozen by the A0 contract
//! (`doc/implementation/router-rust-migration-a0-contract.md`). The typed
//! authority for the schema lives in `skiff-deployment::projection::actor_routing`;
//! this file deliberately uses a TEST-ONLY mirror with `deny_unknown_fields`
//! so the corpus can be verified inside `skiff-runtime-transport` without
//! adding a crate dependency. W-actor must consume `ActorRoutingProjection`
//! itself and pass the same corpus through the real type.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

const CORPUS_SCHEMA_VERSION: &str = "skiff-router-rust-actor-routing-corpus-v1";
const PROJECTION_SCHEMA_VERSION: &str = "skiff-actor-routing-projection-v1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefMirror {
    #[serde(rename = "serviceId")]
    service_id: String,
    #[serde(rename = "actorAbiIdentity")]
    actor_abi_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeploymentMirror {
    #[serde(rename = "serviceId")]
    service_id: String,
    #[serde(rename = "contractVersion")]
    contract_version: String,
    #[serde(rename = "deploymentRevision")]
    deployment_revision: String,
    #[serde(rename = "deploymentArtifactIdentity")]
    deployment_artifact_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackageMirror {
    #[serde(rename = "packageId")]
    package_id: String,
    #[serde(rename = "packageVersion")]
    package_version: String,
    #[serde(rename = "packageBuildId")]
    package_build_id: String,
    #[serde(rename = "packageLocalAbiIdentity")]
    package_local_abi_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MethodMirror {
    actor: RefMirror,
    #[serde(rename = "actorImplementationIdentity")]
    actor_implementation_identity: String,
    #[serde(rename = "methodIdentity")]
    method_identity: String,
    deployment: DeploymentMirror,
    package: PackageMirror,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectionMirror {
    #[serde(rename = "schemaVersion")]
    schema_version: String,
    methods: Vec<MethodMirror>,
}

impl ProjectionMirror {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != PROJECTION_SCHEMA_VERSION {
            return Err(format!(
                "unsupported schemaVersion {}",
                self.schema_version
            ));
        }
        for method in &self.methods {
            if method.actor.service_id != method.deployment.service_id {
                return Err("actor.serviceId must equal deployment.serviceId".to_string());
            }
            validate_identity(
                &method.actor.actor_abi_identity,
                "skiff-actor-abi-v1:sha256",
                "actor.actorAbiIdentity",
            )?;
            validate_identity(
                &method.actor_implementation_identity,
                "skiff-actor-implementation-v1:sha256",
                "actorImplementationIdentity",
            )?;
            validate_identity(
                &method.method_identity,
                "skiff-actor-method-v1:sha256",
                "methodIdentity",
            )?;
            validate_identity(
                &method.deployment.deployment_artifact_identity,
                "skiff-deployment-artifact-v4:sha256",
                "deployment.deploymentArtifactIdentity",
            )?;
            validate_identity(
                &method.package.package_build_id,
                "skiff-package-build-v10:sha256",
                "package.packageBuildId",
            )?;
            validate_identity(
                &method.package.package_local_abi_identity,
                "skiff-package-local-abi-v7:sha256",
                "package.packageLocalAbiIdentity",
            )?;
        }
        let mut seen = std::collections::BTreeSet::new();
        for method in &self.methods {
            if !seen.insert(method.clone()) {
                return Err("duplicate method entry".to_string());
            }
        }
        Ok(())
    }

    fn sorted(&self) -> Vec<MethodMirror> {
        let mut methods = self.methods.clone();
        methods.sort();
        methods
    }
}

fn validate_identity(value: &str, prefix: &str, label: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix(&format!("{prefix}:")) else {
        return Err(format!("{label} must use {prefix}"));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{label} must contain a lowercase sha256 digest"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MethodKey {
    service_id: String,
    actor_abi_identity: String,
    actor_implementation_identity: String,
    method_identity: String,
}

impl From<&MethodMirror> for MethodKey {
    fn from(method: &MethodMirror) -> Self {
        Self {
            service_id: method.actor.service_id.clone(),
            actor_abi_identity: method.actor.actor_abi_identity.clone(),
            actor_implementation_identity: method.actor_implementation_identity.clone(),
            method_identity: method.method_identity.clone(),
        }
    }
}

/// Stateless typed query over an immutable A0 projection (C-actor §2).
///
/// The view never receives File IR coordinates (modulePath/actorName/
/// methodName/sourceSpan/unit/file), never owns an index or refresh, and
/// returns only the exact method entry with its deployment/package binding.
#[derive(Debug, Clone)]
struct CatalogView {
    projection: ProjectionMirror,
    by_key: BTreeMap<MethodKey, MethodMirror>,
}

impl CatalogView {
    fn new(projection: ProjectionMirror) -> Self {
        let by_key = projection
            .methods
            .iter()
            .map(|method| (MethodKey::from(method), method.clone()))
            .collect();
        Self { projection, by_key }
    }

    fn has_method(&self, key: &MethodKey) -> bool {
        self.by_key.contains_key(key)
    }

    fn method_for(&self, key: &MethodKey) -> Option<&MethodMirror> {
        self.by_key.get(key)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PositiveCase {
    id: String,
    json: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NegativeCase {
    id: String,
    #[serde(rename = "rejectAt")]
    reject_at: String,
    reason: String,
    json: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    #[serde(rename = "schemaVersion")]
    schema_version: String,
    #[serde(rename = "sourceContract")]
    source_contract: String,
    positive: Vec<PositiveCase>,
    negative: Vec<NegativeCase>,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../testdata/actor-routing-projection.json"
    ))
    .expect("actor routing projection corpus must decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_is_frozen() {
        let corpus = corpus();
        assert_eq!(corpus.schema_version, CORPUS_SCHEMA_VERSION);
        assert_eq!(
            corpus.source_contract,
            "router-rust-migration-a0-contract.md"
        );
        assert!(!corpus.positive.is_empty());
        assert!(!corpus.negative.is_empty());
    }

    #[test]
    fn positive_projections_parse_validate_and_sort() {
        let corpus = corpus();
        for case in &corpus.positive {
            let projection: ProjectionMirror = serde_json::from_value(case.json.clone())
                .unwrap_or_else(|error| panic!("{}: {error}", case.id));
            projection
                .validate()
                .unwrap_or_else(|error| panic!("{}: {error}", case.id));
            assert_eq!(
                projection.schema_version,
                PROJECTION_SCHEMA_VERSION,
                "{}",
                case.id
            );
            assert_eq!(
                projection.methods,
                projection.sorted(),
                "{}: methods must be sorted by typed key",
                case.id
            );
        }
    }

    #[test]
    fn empty_projection_is_valid_and_has_no_methods() {
        let corpus = corpus();
        let empty = corpus
            .positive
            .iter()
            .find(|case| case.id == "empty-methods")
            .expect("empty-methods case");
        let projection: ProjectionMirror =
            serde_json::from_value(empty.json.clone()).expect("empty projection parses");
        projection.validate().expect("empty projection validates");
        assert!(projection.methods.is_empty());
        let view = CatalogView::new(projection);
        assert!(!view.has_method(&MethodKey {
            service_id: "example.com/docs".to_string(),
            actor_abi_identity: "skiff-actor-abi-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            actor_implementation_identity:
                "skiff-actor-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            method_identity:
                "skiff-actor-method-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
        }));
    }

    #[test]
    fn negative_projections_fail_closed() {
        let corpus = corpus();
        for case in &corpus.negative {
            match case.reject_at.as_str() {
                "deserialize" => {
                    let result: Result<ProjectionMirror, _> =
                        serde_json::from_value(case.json.clone());
                    assert!(
                        result.is_err(),
                        "{}: projection with {} must be rejected at deserialize",
                        case.id,
                        case.reason
                    );
                }
                "validate" => {
                    let projection: ProjectionMirror = serde_json::from_value(case.json.clone())
                        .unwrap_or_else(|error| panic!("{}: {error}", case.id));
                    let error = projection.validate().expect_err(&format!(
                        "{}: projection with {} must be rejected at validate",
                        case.id, case.reason
                    ));
                    assert!(!error.is_empty(), "{}: validation error must be non-empty", case.id);
                }
                other => panic!("{}: unknown rejectAt {other}", case.id),
            }
        }
    }

    #[test]
    fn file_ir_coordinates_are_structurally_absent_from_the_catalog_view() {
        // The mirror struct has no fields for source, File IR coordinates or
        // executable payload. The corpus negative cases prove the serde
        // boundary rejects them; this test proves the catalog view query key
        // contains only projection identities.
        let corpus = corpus();
        let first = corpus.positive.first().expect("positive case");
        let projection: ProjectionMirror =
            serde_json::from_value(first.json.clone()).expect("projection parses");
        let view = CatalogView::new(projection);
        let method = view
            .projection
            .methods
            .first()
            .expect("projection has a method");
        let key = MethodKey::from(method);
        assert!(view.has_method(&key));
        let entry = view.method_for(&key).expect("method_for resolves");
        assert_eq!(entry.actor.service_id, method.actor.service_id);
        assert_eq!(entry.deployment.service_id, method.deployment.service_id);
        assert_eq!(entry.package.package_id, method.package.package_id);
        assert!(view.method_for(&MethodKey {
            service_id: "example.com/unknown".to_string(),
            actor_abi_identity: method.actor.actor_abi_identity.clone(),
            actor_implementation_identity: method.actor_implementation_identity.clone(),
            method_identity: method.method_identity.clone(),
        })
        .is_none());
    }

    #[test]
    fn duplicate_and_wrong_schema_are_rejected() {
        let corpus = corpus();
        for id in ["unsupported-schema-version", "duplicate-method-entry"] {
            let case = corpus
                .negative
                .iter()
                .find(|case| case.id == id)
                .unwrap_or_else(|| panic!("negative case {id}"));
            let projection: ProjectionMirror = serde_json::from_value(case.json.clone())
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            assert!(projection.validate().is_err(), "{id} must fail validation");
        }
    }
}
