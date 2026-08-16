use std::collections::BTreeMap;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    derive_synthetic_callback_callable_id, BytecodeFunctionOrigin, PackageArtifact,
    PackageArtifactRef, PackageCallableId, PackageExecutableCoordinate,
};

use super::{manifest_error, manifest_mismatch, CanonicalImplementationCallableIndex};
use crate::deployment_bytecode::{
    DeploymentBytecodeHydrationError, DeploymentBytecodeManifestKind,
};

type SyntheticSite = (PackageExecutableCoordinate, u32);

#[derive(Debug)]
pub(super) struct SyntheticCallbackIndex {
    by_site: BTreeMap<SyntheticSite, SyntheticCallbackBinding>,
    by_callable: BTreeMap<PackageCallableId, String>,
    by_function: BTreeMap<String, PackageCallableId>,
}

#[derive(Debug)]
struct SyntheticCallbackBinding {
    function_key: String,
    callable: PackageCallableId,
}

impl SyntheticCallbackIndex {
    pub(super) fn checked(
        reference: &PackageArtifactRef,
        artifact: &PackageArtifact,
        bytecode: &ValidatedBytecodeArtifact,
        ordinary_functions: &BTreeMap<PackageExecutableCoordinate, String>,
        canonical_ordinary: &CanonicalImplementationCallableIndex,
    ) -> Result<Self, DeploymentBytecodeHydrationError> {
        let mut functions_by_site = BTreeMap::new();
        for function in bytecode.view().functions() {
            let BytecodeFunctionOrigin::SyntheticCallback {
                owner,
                site_ordinal,
            } = &function.origin
            else {
                continue;
            };
            if !ordinary_functions.contains_key(owner) {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic callback function {:?} owner {owner:?} has no ordinary admitted function",
                        function.function_key
                    ),
                );
            }
            let site = (owner.clone(), *site_ordinal);
            if let Some(previous) =
                functions_by_site.insert(site.clone(), function.function_key.clone())
            {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic callback site {site:?} is owned by both {previous:?} and {:?}",
                        function.function_key
                    ),
                );
            }
        }

        let mut expected_by_site = BTreeMap::new();
        for row in &artifact.synthetic_callback_owners {
            let site = (row.owner.clone(), row.site_ordinal);
            if expected_by_site.insert(site.clone(), row).is_some() {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!("package callback owner manifest repeats site {site:?}"),
                );
            }
            if row.interface.interface_abi_id.is_empty()
                || row.method_abi_id.is_empty()
                || row.contract.package_id.is_empty()
                || row.contract.stable_schema_key.is_empty()
                || row.contract.package_schema_type_id.as_str().is_empty()
            {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic callback site {site:?} lacks exact interface method or contract facts"
                    ),
                );
            }
        }
        if functions_by_site.keys().ne(expected_by_site.keys()) {
            return manifest_error(
                reference,
                DeploymentBytecodeManifestKind::SyntheticCallback,
                format!(
                    "synthetic bytecode sites {:?} do not exact-cover package owner sites {:?}",
                    functions_by_site.keys().collect::<Vec<_>>(),
                    expected_by_site.keys().collect::<Vec<_>>()
                ),
            );
        }

        let functions = bytecode
            .view()
            .functions()
            .iter()
            .map(|function| (function.function_key.as_str(), function))
            .collect::<BTreeMap<_, _>>();
        let mut index = Self {
            by_site: BTreeMap::new(),
            by_callable: BTreeMap::new(),
            by_function: BTreeMap::new(),
        };
        for (site, row) in expected_by_site {
            let ordinary_owner = canonical_ordinary
                .by_executable
                .get(&row.owner)
                .ok_or_else(|| {
                    manifest_mismatch(
                        reference,
                        DeploymentBytecodeManifestKind::SyntheticCallback,
                        format!("synthetic callback site {site:?} has no canonical ordinary owner"),
                    )
                })?;
            let expected_callable = derive_synthetic_callback_callable_id(
                &artifact.package_id,
                ordinary_owner,
                row.site_ordinal,
            )
            .map_err(|error| {
                manifest_mismatch(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!("synthetic callback site {site:?} identity cannot be derived: {error}"),
                )
            })?;
            if row.package_callable_id != expected_callable {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic callback site {site:?} declares {}, expected {expected_callable}",
                        row.package_callable_id
                    ),
                );
            }

            let function_key = functions_by_site.get(&site).ok_or_else(|| {
                manifest_mismatch(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!("synthetic callback site {site:?} disappeared after exact cover"),
                )
            })?;
            let function = functions
                .get(function_key.as_str())
                .copied()
                .ok_or_else(|| {
                    manifest_mismatch(
                        reference,
                        DeploymentBytecodeManifestKind::SyntheticCallback,
                        format!(
                            "synthetic callback function {function_key:?} is absent from the admitted view"
                        ),
                    )
                })?;
            if function.effect_summary_ref != row.package_callable_id {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic function {function_key:?} effectSummaryRef {} disagrees with package owner {}",
                        function.effect_summary_ref, row.package_callable_id
                    ),
                );
            }
            if !artifact
                .callable_semantic_facts
                .contains_key(&row.package_callable_id)
            {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic callable {} has no package callableSemanticFacts row",
                        row.package_callable_id
                    ),
                );
            }

            if let Some(previous) = index
                .by_callable
                .insert(row.package_callable_id.clone(), function_key.clone())
            {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic callable {} owns both {previous:?} and {function_key:?}",
                        row.package_callable_id
                    ),
                );
            }
            if let Some(previous) = index
                .by_function
                .insert(function_key.clone(), row.package_callable_id.clone())
            {
                return manifest_error(
                    reference,
                    DeploymentBytecodeManifestKind::SyntheticCallback,
                    format!(
                        "synthetic function {function_key:?} has callable owners {previous} and {}",
                        row.package_callable_id
                    ),
                );
            }
            index.by_site.insert(
                site,
                SyntheticCallbackBinding {
                    function_key: function_key.clone(),
                    callable: row.package_callable_id.clone(),
                },
            );
        }
        Ok(index)
    }

    pub(super) fn function_key_for_site(
        &self,
        owner: &PackageExecutableCoordinate,
        site_ordinal: u32,
    ) -> Option<&str> {
        self.by_site
            .get(&(owner.clone(), site_ordinal))
            .map(|binding| binding.function_key.as_str())
    }

    pub(super) fn callable_for_site(
        &self,
        owner: &PackageExecutableCoordinate,
        site_ordinal: u32,
    ) -> Option<&PackageCallableId> {
        self.by_site
            .get(&(owner.clone(), site_ordinal))
            .map(|binding| &binding.callable)
    }

    pub(super) fn function_key_for_callable(&self, callable: &PackageCallableId) -> Option<&str> {
        self.by_callable.get(callable).map(String::as_str)
    }

    pub(super) fn callable_for_function(&self, function_key: &str) -> Option<&PackageCallableId> {
        self.by_function.get(function_key)
    }
}
