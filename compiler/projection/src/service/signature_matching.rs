use std::collections::BTreeMap;

use skiff_artifact_model::cross_package_identity::{
    canonicalize_reexport_type_ref, PackageReexportIndex,
};
use skiff_artifact_model::{FunctionTypeParamIr, PackageRefIr, ParamIr, TypeRefIr};

use crate::{context::ProjectedPackageDependency, typed_artifacts::PackageUnit};

#[derive(Debug, Clone, Default)]
pub struct SignatureTypeRefContext {
    package_ids_by_ref: BTreeMap<String, String>,
    /// Loaded package units, used to resolve transitive cross-package alias
    /// re-exports so the same underlying type encoded via different dependency
    /// paths compares equal (e.g. `agent.llm.LlmRequest` re-exporting
    /// `llmApi.LlmRequest`). Empty when the context is built without package
    /// units (matching is then exact-by-ref via `package_ids_by_ref`).
    package_units: Vec<PackageUnit>,
}

impl SignatureTypeRefContext {
    pub fn from_package_dependencies(dependencies: &[ProjectedPackageDependency]) -> Self {
        Self::from_dependencies_and_units(dependencies, &[])
    }

    /// Build a context that can also resolve transitive package alias re-exports
    /// using the loaded `package_units`.
    pub fn from_dependencies_and_units(
        dependencies: &[ProjectedPackageDependency],
        package_units: &[PackageUnit],
    ) -> Self {
        let mut package_ids_by_ref = BTreeMap::new();
        for dependency in dependencies {
            package_ids_by_ref.insert(dependency.id.clone(), dependency.id.clone());
            package_ids_by_ref.insert(
                dependency.effective_alias().to_string(),
                dependency.id.clone(),
            );
        }
        Self {
            package_ids_by_ref,
            package_units: package_units.to_vec(),
        }
    }

    fn package_ref_identity<'a>(&'a self, package: &'a PackageRefIr) -> &'a str {
        match package {
            PackageRefIr::PackageId { package_id } => package_id,
            PackageRefIr::Dependency { dependency_ref } => self
                .package_ids_by_ref
                .get(dependency_ref)
                .map(String::as_str)
                .unwrap_or(dependency_ref),
        }
    }

    /// Canonicalize a type ref by resolving transitive cross-package alias
    /// re-exports to their underlying `PackageId`-based package symbol. A no-op
    /// when no package units are available or the ref is not a cross-package
    /// alias re-export.
    fn canonicalize_reexport(&self, ty: &TypeRefIr) -> TypeRefIr {
        if self.package_units.is_empty() {
            return ty.clone();
        }
        let index = PackageReexportIndex::new(self.package_units.iter());
        canonicalize_reexport_type_ref(ty, &self.package_ids_by_ref, &index)
    }
}

pub fn executable_params_match_interface_method(
    actual: &[ParamIr],
    expected: &[FunctionTypeParamIr],
    signature_context: &SignatureTypeRefContext,
) -> bool {
    let actual = executable_signature_params(actual);
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| {
                actual.name == expected.name
                    && signature_type_ref_matches(&actual.ty, &expected.ty, signature_context)
            })
}

pub fn executable_signature_params(actual: &[ParamIr]) -> &[ParamIr] {
    if actual.first().is_some_and(|param| param.name == "self") {
        actual.get(1..).unwrap_or(&[])
    } else {
        actual
    }
}

pub fn signature_type_ref_matches(
    actual: &TypeRefIr,
    expected: &TypeRefIr,
    signature_context: &SignatureTypeRefContext,
) -> bool {
    // Resolve transitive cross-package alias re-exports on both sides first so
    // the same underlying type encoded via different dependency paths (e.g.
    // `agent.llm.LlmRequest` re-exporting `llmApi.LlmRequest`) compares equal.
    // Canonicalization is idempotent, so the recursive structural match below
    // (which re-checks inner nodes) stays correct.
    let actual = signature_context.canonicalize_reexport(actual);
    let expected = signature_context.canonicalize_reexport(expected);
    signature_type_ref_matches_canonical(&actual, &expected, signature_context)
}

fn signature_type_ref_matches_canonical(
    actual: &TypeRefIr,
    expected: &TypeRefIr,
    signature_context: &SignatureTypeRefContext,
) -> bool {
    if actual == expected {
        return true;
    }
    match (actual, expected) {
        (
            TypeRefIr::PackageSymbol {
                symbol: actual_symbol,
            },
            TypeRefIr::PackageSymbol {
                symbol: expected_symbol,
            },
        ) => {
            package_symbol_paths_match_for_signature(
                &actual_symbol.symbol_path,
                &expected_symbol.symbol_path,
            ) && actual_symbol.abi_expectation == expected_symbol.abi_expectation
                && package_refs_match_for_signature(
                    &actual_symbol.package,
                    &expected_symbol.package,
                    signature_context,
                )
        }
        (
            TypeRefIr::PackageSymbol {
                symbol: actual_symbol,
            },
            TypeRefIr::ServiceSymbol {
                symbol: expected_symbol,
            },
        ) => package_symbol_paths_match_for_signature(
            &actual_symbol.symbol_path,
            &expected_symbol.symbol_path(),
        ),
        (
            TypeRefIr::ServiceSymbol {
                symbol: actual_symbol,
            },
            TypeRefIr::PackageSymbol {
                symbol: expected_symbol,
            },
        ) => package_symbol_paths_match_for_signature(
            &actual_symbol.symbol_path(),
            &expected_symbol.symbol_path,
        ),
        (
            TypeRefIr::Native {
                name: actual_name,
                args: actual_args,
            },
            TypeRefIr::Native {
                name: expected_name,
                args: expected_args,
            },
        ) => {
            actual_name == expected_name
                && actual_args.len() == expected_args.len()
                && actual_args
                    .iter()
                    .zip(expected_args.iter())
                    .all(|(actual, expected)| {
                        signature_type_ref_matches(actual, expected, signature_context)
                    })
        }
        (
            TypeRefIr::Record {
                fields: actual_fields,
            },
            TypeRefIr::Record {
                fields: expected_fields,
            },
        ) => {
            actual_fields.len() == expected_fields.len()
                && actual_fields.iter().all(|(name, actual)| {
                    expected_fields.get(name).is_some_and(|expected| {
                        signature_type_ref_matches(actual, expected, signature_context)
                    })
                })
        }
        (
            TypeRefIr::Union {
                items: actual_items,
            },
            TypeRefIr::Union {
                items: expected_items,
            },
        ) => {
            actual_items.len() == expected_items.len()
                && actual_items
                    .iter()
                    .zip(expected_items.iter())
                    .all(|(actual, expected)| {
                        signature_type_ref_matches(actual, expected, signature_context)
                    })
        }
        (TypeRefIr::Nullable { inner: actual }, TypeRefIr::Nullable { inner: expected }) => {
            signature_type_ref_matches(actual, expected, signature_context)
        }
        (
            TypeRefIr::Function {
                params: actual_params,
                return_type: actual_return,
            },
            TypeRefIr::Function {
                params: expected_params,
                return_type: expected_return,
            },
        ) => {
            actual_params.len() == expected_params.len()
                && actual_params
                    .iter()
                    .zip(expected_params.iter())
                    .all(|(actual, expected)| {
                        actual.name == expected.name
                            && signature_type_ref_matches(
                                &actual.ty,
                                &expected.ty,
                                signature_context,
                            )
                    })
                && signature_type_ref_matches(actual_return, expected_return, signature_context)
        }
        _ => false,
    }
}

fn package_symbol_paths_match_for_signature(actual: &str, expected: &str) -> bool {
    actual == expected
        || strip_package_public_api_prefix(actual) == Some(expected)
        || strip_package_public_api_prefix(expected) == Some(actual)
        || strip_package_public_api_prefix(actual)
            .zip(strip_package_public_api_prefix(expected))
            .is_some_and(|(actual, expected)| actual == expected)
}

fn strip_package_public_api_prefix(symbol_path: &str) -> Option<&str> {
    symbol_path.split_once('.').map(|(_, rest)| rest)
}

fn package_refs_match_for_signature(
    actual: &PackageRefIr,
    expected: &PackageRefIr,
    signature_context: &SignatureTypeRefContext,
) -> bool {
    signature_context.package_ref_identity(actual)
        == signature_context.package_ref_identity(expected)
}
