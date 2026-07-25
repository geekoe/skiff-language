use std::{collections::BTreeMap, fmt};

use skiff_artifact_model::PackageCallableSignature;

use crate::ProjectionExecutableKey;

/// Normalize one source API path into the package-scoped path used by both
/// compiled handoff keys and PackageArtifact callable targets.
pub fn canonical_package_public_path(package_id: &str, public_path: &str) -> String {
    if package_id == skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID
        && !public_path.starts_with("std.")
    {
        format!("std.{public_path}")
    } else {
        public_path.to_string()
    }
}

/// Stable key for one package API callable. Public path remains part of the
/// key because two API entries may intentionally select the same executable
/// while exposing distinct typed signatures.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectionPackageCallableKey {
    public_path: String,
    executable: ProjectionExecutableKey,
}

impl ProjectionPackageCallableKey {
    pub fn new(
        public_path: impl Into<String>,
        module_path: impl Into<String>,
        executable_index: u32,
    ) -> Self {
        Self {
            public_path: public_path.into(),
            executable: ProjectionExecutableKey::new(module_path, executable_index),
        }
    }

    pub fn public_path(&self) -> &str {
        &self.public_path
    }

    pub fn executable(&self) -> &ProjectionExecutableKey {
        &self.executable
    }
}

impl fmt::Display for ProjectionPackageCallableKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} ({}#{})",
            self.public_path,
            self.executable.module_path(),
            self.executable.executable_index()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateProjectionPackageCallableSignature {
    key: ProjectionPackageCallableKey,
}

impl DuplicateProjectionPackageCallableSignature {
    pub fn key(&self) -> &ProjectionPackageCallableKey {
        &self.key
    }
}

impl fmt::Display for DuplicateProjectionPackageCallableSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duplicate package callable signature for {}",
            self.key
        )
    }
}

impl std::error::Error for DuplicateProjectionPackageCallableSignature {}

/// Canonical typed signature handoff for PackageArtifact projection.
///
/// PackageArtifact projection never reconstructs these signatures from File
/// IR. The producer must provide an exact entry for every package API callable.
#[derive(Debug, Clone, Default)]
pub struct ProjectionPackageCallableSignatureFacts {
    signatures: BTreeMap<ProjectionPackageCallableKey, PackageCallableSignature>,
}

impl ProjectionPackageCallableSignatureFacts {
    pub fn try_from_entries(
        entries: impl IntoIterator<Item = (ProjectionPackageCallableKey, PackageCallableSignature)>,
    ) -> Result<Self, DuplicateProjectionPackageCallableSignature> {
        let mut signatures = BTreeMap::new();
        for (key, signature) in entries {
            if signatures.insert(key.clone(), signature).is_some() {
                return Err(DuplicateProjectionPackageCallableSignature { key });
            }
        }
        Ok(Self { signatures })
    }

    pub fn signature(
        &self,
        key: &ProjectionPackageCallableKey,
    ) -> Option<&PackageCallableSignature> {
        self.signatures.get(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &ProjectionPackageCallableKey> {
        self.signatures.keys()
    }

    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        PackageCallableParameter, PackageCallableSignature, PackageSchemaTypeId, PackageTypeRef,
        TypeRefIr,
    };

    use super::*;

    #[test]
    fn package_public_path_normalization_has_one_std_scope() {
        assert_eq!(
            canonical_package_public_path(skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID, "io"),
            "std.io"
        );
        assert_eq!(
            canonical_package_public_path(
                skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID,
                "std.io"
            ),
            "std.io"
        );
        assert_eq!(canonical_package_public_path("example.com/pkg", "io"), "io");
    }

    #[test]
    fn duplicate_callable_signature_key_is_rejected() {
        let key = ProjectionPackageCallableKey::new("run", "api", 0);
        let signature = PackageCallableSignature {
            parameters: Vec::new(),
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            may_suspend: false,
        };
        let error = ProjectionPackageCallableSignatureFacts::try_from_entries([
            (key.clone(), signature.clone()),
            (key.clone(), signature),
        ])
        .expect_err("duplicate key must fail closed");
        assert_eq!(error.key(), &key);
    }

    #[test]
    fn callable_signature_wire_contains_only_open_error_surface() {
        let signature = PackageCallableSignature {
            parameters: Vec::new(),
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("void"),
            },
            may_suspend: false,
        };
        let wire = serde_json::to_value(&signature).unwrap();

        assert_eq!(
            wire.as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["parameters", "returnType", "maySuspend"]
        );
        assert!(wire.get("throwTypes").is_none());

        let mut legacy = wire;
        legacy["throwTypes"] = serde_json::json!([]);
        assert!(serde_json::from_value::<PackageCallableSignature>(legacy).is_err());
    }

    #[test]
    fn projection_input_preserves_exact_nested_package_schema_signature() {
        let key = ProjectionPackageCallableKey::new("submit", "api", 4);
        let contract = PackageTypeRef::PackageSchema {
            package_id: "example.com/models".to_string(),
            stable_schema_key: "api.User".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("package-type:models:User"),
        };
        let signature = PackageCallableSignature {
            parameters: vec![PackageCallableParameter {
                name: "users".to_string(),
                ty: PackageTypeRef::Nullable {
                    inner: Box::new(PackageTypeRef::Container {
                        name: "Array".to_string(),
                        arguments: vec![contract.clone()],
                    }),
                },
            }],
            return_type: contract,
            may_suspend: true,
        };
        let facts = ProjectionPackageCallableSignatureFacts::try_from_entries([(
            key.clone(),
            signature.clone(),
        )])
        .unwrap();
        let input = crate::ProjectionInput::new(
            Vec::new(),
            Vec::new(),
            crate::ProjectionSourceFacts::default(),
            crate::ProjectionLoweringFacts::default(),
            facts,
        );

        assert_eq!(
            input.view().callable_signatures().signature(&key),
            Some(&signature)
        );
    }
}
