//! Cross-package type identity canonicalization for binding-requirement
//! signature matching.
//!
//! The same underlying package type can be referenced through different
//! dependency paths. For example, the `example.com/agent` package declares
//! `alias LlmRequest = llmApi.LlmRequest` (re-exporting `example.com/llm-api`'s
//! `LlmRequest`), and a consuming service can refer to it either directly as
//! `llmApi.LlmRequest` (`PackageSymbol{example.com/llm-api, "LlmRequest"}`) or
//! transitively as `agent.llm.LlmRequest`
//! (`PackageSymbol{agent, "llm.LlmRequest"}` -> alias -> the same underlying
//! type). A binding requirement's published method signature and the bound
//! implementation's signature can therefore encode the SAME type two different
//! ways, breaking the exact-equality binding-requirement signature match.
//!
//! [`canonicalize_reexport_type_ref`] resolves a [`TypeRefIr`] to a stable
//! canonical form by:
//! - rebasing every `PackageSymbol` package ref to a concrete `PackageId`, and
//! - following package-exported alias re-exports to their underlying package
//!   symbol (transitively, with cycle protection).
//!
//! Applied to BOTH operands of a binding-requirement signature comparison, the
//! two encodings of one type collapse to identical bytes. It is a structural
//! no-op for refs that are not cross-package alias re-exports, so it does not
//! change matching for unrelated types.

use std::collections::{BTreeMap, BTreeSet};

use crate::package_unit::PackageUnit;
use crate::symbols::{PackageRefIr, PackageSymbolRef};
use crate::types::{TypeDescriptorIr, TypeRefIr};

/// Read-only view over the loaded package units needed to resolve cross-package
/// alias re-exports. Construct once per match scope from the loaded package set.
pub struct PackageReexportIndex<'a> {
    packages_by_id: BTreeMap<&'a str, &'a PackageUnit>,
}

impl<'a> PackageReexportIndex<'a> {
    pub fn new(packages: impl IntoIterator<Item = &'a PackageUnit>) -> Self {
        Self {
            packages_by_id: packages
                .into_iter()
                .map(|package| (package.package_id.as_str(), package))
                .collect(),
        }
    }

    fn package(&self, package_id: &str) -> Option<&'a PackageUnit> {
        self.packages_by_id.get(package_id).copied()
    }

    /// Map a dependency ref within `owner_package_id`'s namespace to a package
    /// id. Returns `None` if the owner package or the dependency is unknown.
    fn dependency_package_id(
        &self,
        owner_package_id: &str,
        dependency_ref: &str,
    ) -> Option<&'a str> {
        let owner = self.package(owner_package_id)?;
        owner
            .dependencies
            .iter()
            .find(|dependency| {
                dependency.alias == dependency_ref || dependency.id == dependency_ref
            })
            .map(|dependency| dependency.id.as_str())
    }

    /// Resolve a package symbol's package ref to a concrete package id, given the
    /// namespace (`owner_package_id`) the ref is expressed in. `None` for the
    /// consumer/service namespace where dependency refs are resolved via
    /// `consumer_deps` instead.
    fn package_id_for_ref(
        &self,
        package: &PackageRefIr,
        owner_package_id: Option<&str>,
        consumer_deps: &BTreeMap<String, String>,
    ) -> Option<String> {
        match package {
            PackageRefIr::PackageId { package_id } => Some(package_id.clone()),
            PackageRefIr::Dependency { dependency_ref } => match owner_package_id {
                Some(owner) => self
                    .dependency_package_id(owner, dependency_ref)
                    .map(ToString::to_string),
                None => consumer_deps.get(dependency_ref).cloned(),
            },
        }
    }

    /// Look up a package type export's alias target by export symbol path. The
    /// package's `implementation_links.types` map is keyed by the exact export
    /// path (e.g. `"llm.LlmRequest"`); each `TypeExport` carries the type
    /// descriptor, so an alias re-export is recognized directly. Returns `None`
    /// for non-alias exports or unknown paths.
    fn alias_target(&self, package_id: &str, symbol_path: &str) -> Option<&'a TypeRefIr> {
        let package = self.package(package_id)?;
        let export = package.implementation_links.types.get(symbol_path)?;
        match export.descriptor.as_ref()? {
            TypeDescriptorIr::Alias { target } => Some(target),
            _ => None,
        }
    }
}

fn symbol_paths_match(a: &str, b: &str) -> bool {
    a == b || trailing_segment(a) == trailing_segment(b)
}

fn trailing_segment(symbol_path: &str) -> &str {
    symbol_path.rsplit('.').next().unwrap_or(symbol_path)
}

/// Canonicalize a type ref by rebasing package refs to `PackageId` and following
/// package alias re-exports to their underlying package symbol.
///
/// `consumer_deps` maps the consuming unit's dependency refs to package ids
/// (e.g. the sample service's `agent -> example.com/agent`, `llmApi ->
/// example.com/llm-api`); top-level `Dependency` refs are resolved through it.
pub fn canonicalize_reexport_type_ref(
    ty: &TypeRefIr,
    consumer_deps: &BTreeMap<String, String>,
    index: &PackageReexportIndex<'_>,
) -> TypeRefIr {
    canonicalize_inner(ty, None, consumer_deps, index)
}

fn canonicalize_inner(
    ty: &TypeRefIr,
    owner_package_id: Option<&str>,
    consumer_deps: &BTreeMap<String, String>,
    index: &PackageReexportIndex<'_>,
) -> TypeRefIr {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            canonicalize_package_symbol(symbol, owner_package_id, consumer_deps, index)
        }
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| canonicalize_inner(arg, owner_package_id, consumer_deps, index))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(canonicalize_inner(
                inner,
                owner_package_id,
                consumer_deps,
                index,
            )),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| canonicalize_inner(item, owner_package_id, consumer_deps, index))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field_ty)| {
                    (
                        name.clone(),
                        canonicalize_inner(field_ty, owner_package_id, consumer_deps, index),
                    )
                })
                .collect(),
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| crate::types::FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: canonicalize_inner(&param.ty, owner_package_id, consumer_deps, index),
                })
                .collect(),
            return_type: Box::new(canonicalize_inner(
                return_type,
                owner_package_id,
                consumer_deps,
                index,
            )),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: crate::publication_abi::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| canonicalize_inner(arg, owner_package_id, consumer_deps, index))
                    .collect(),
            },
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

fn canonicalize_package_symbol(
    symbol: &PackageSymbolRef,
    owner_package_id: Option<&str>,
    consumer_deps: &BTreeMap<String, String>,
    index: &PackageReexportIndex<'_>,
) -> TypeRefIr {
    // Resolve the package ref to a concrete package id in the current namespace.
    let Some(mut package_id) =
        index.package_id_for_ref(&symbol.package, owner_package_id, consumer_deps)
    else {
        return TypeRefIr::PackageSymbol {
            symbol: symbol.clone(),
        };
    };
    let mut symbol_path = symbol.symbol_path.clone();
    let abi_expectation = symbol.abi_expectation.clone();
    let mut visited = BTreeSet::new();

    // Follow alias re-exports to the underlying package symbol. Each hop reads
    // the alias target, which is expressed in the ALIAS-OWNER package's
    // namespace, so dependency refs in the target are resolved via that
    // package's own dependency map.
    while visited.insert((package_id.clone(), symbol_path.clone())) {
        let Some(target) = index.alias_target(&package_id, &symbol_path) else {
            break;
        };
        let TypeRefIr::PackageSymbol {
            symbol: target_symbol,
        } = target
        else {
            // Alias to a non-package type (e.g. a local/native type): stop and
            // keep the current package-relative form.
            break;
        };
        let Some(next_package_id) =
            index.package_id_for_ref(&target_symbol.package, Some(&package_id), consumer_deps)
        else {
            break;
        };
        // A self-referential alias entry (target == itself) is the package's own
        // canonical record for the symbol; stop without changing the symbol path.
        if next_package_id == package_id
            && symbol_paths_match(&target_symbol.symbol_path, &symbol_path)
        {
            break;
        }
        package_id = next_package_id;
        symbol_path = target_symbol.symbol_path.clone();
    }

    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId { package_id },
            symbol_path,
            abi_expectation,
        },
    }
}
