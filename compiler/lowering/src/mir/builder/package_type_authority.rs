use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{ExternalRefTable, PackageRefIr, PackageSymbolRef, TypeRefIr};

/// Projects a dependency-relative package type onto a canonical package owner
/// only when MIR already owns a closed, order-independent proof of that join.
///
/// The proof is deliberately stronger than an alias or ABI lookup: the
/// dependency occurrence has a non-empty ABI, that ABI names exactly one
/// PackageId anywhere in the external-ref table, and the alias/package-id
/// record declarations for the same symbol path are recursively identical.
/// Missing or conflicting evidence produces no row, so admission remains fail
/// closed.
pub(super) struct PackageTypeProjection {
    pub external_refs: ExternalRefTable,
    pub package_type_records: BTreeMap<(String, String), BTreeMap<String, TypeRefIr>>,
}

pub(super) fn project_package_type_authority(
    external_refs: &ExternalRefTable,
    package_type_records: &BTreeMap<(String, String), BTreeMap<String, TypeRefIr>>,
) -> PackageTypeProjection {
    let mut projected = external_refs.clone();
    let package_ids_by_abi = external_refs.package_symbols.iter().fold(
        BTreeMap::<String, BTreeSet<String>>::new(),
        |mut owners, symbol| {
            if let (PackageRefIr::PackageId { package_id }, Some(abi)) =
                (&symbol.package, nonempty_abi(symbol))
            {
                owners
                    .entry(abi.to_string())
                    .or_default()
                    .insert(package_id.clone());
            }
            owners
        },
    );

    let mut additions = BTreeMap::<(String, String, String), PackageSymbolRef>::new();
    for relative in &external_refs.package_symbols {
        let PackageRefIr::Dependency { dependency_ref } = &relative.package else {
            continue;
        };
        let Some(abi) = nonempty_abi(relative) else {
            continue;
        };
        let Some(package_ids) = package_ids_by_abi.get(abi) else {
            continue;
        };
        if package_ids.len() != 1 {
            continue;
        }
        let package_id = package_ids
            .first()
            .expect("one exact ABI package owner was checked");
        let alias_key = (dependency_ref.clone(), relative.symbol_path.clone());
        let package_key = (package_id.clone(), relative.symbol_path.clone());
        let (Some(alias_record), Some(package_record)) = (
            package_type_records.get(&alias_key),
            package_type_records.get(&package_key),
        ) else {
            continue;
        };
        if alias_record != package_record {
            continue;
        }

        let same_path_owners = external_refs
            .package_symbols
            .iter()
            .filter_map(|candidate| {
                if candidate.symbol_path != relative.symbol_path
                    || nonempty_abi(candidate) != Some(abi)
                {
                    return None;
                }
                match &candidate.package {
                    PackageRefIr::PackageId { package_id } => Some(package_id.as_str()),
                    PackageRefIr::Dependency { .. } => None,
                }
            })
            .collect::<BTreeSet<_>>();
        if !same_path_owners.is_empty() && same_path_owners != BTreeSet::from([package_id.as_str()])
        {
            continue;
        }
        if same_path_owners.is_empty() {
            additions.insert(
                (
                    package_id.clone(),
                    relative.symbol_path.clone(),
                    abi.to_string(),
                ),
                PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: package_id.clone(),
                    },
                    symbol_path: relative.symbol_path.clone(),
                    abi_expectation: Some(abi.to_string()),
                },
            );
        }
    }
    projected.package_symbols.extend(additions.into_values());
    let mut projected_records = package_type_records.clone();
    project_recursive_record_closure(&mut projected, &mut projected_records);
    projected
        .package_symbols
        .sort_by(|left, right| package_symbol_sort_key(left).cmp(&package_symbol_sort_key(right)));
    PackageTypeProjection {
        external_refs: projected,
        package_type_records: projected_records,
    }
}

fn project_recursive_record_closure(
    external_refs: &mut ExternalRefTable,
    package_type_records: &mut BTreeMap<(String, String), BTreeMap<String, TypeRefIr>>,
) {
    let package_abis = package_abis(external_refs);
    let mut closure = BTreeMap::<(String, String), String>::new();
    let mut pending = external_refs
        .package_symbols
        .iter()
        .filter_map(|symbol| {
            let PackageRefIr::PackageId { package_id } = &symbol.package else {
                return None;
            };
            let abi = nonempty_abi(symbol)?;
            (package_abis.get(package_id) == Some(&BTreeSet::from([abi.to_string()]))
                && package_type_records
                    .contains_key(&(package_id.clone(), symbol.symbol_path.clone())))
            .then(|| {
                (
                    package_id.clone(),
                    symbol.symbol_path.clone(),
                    abi.to_string(),
                )
            })
        })
        .collect::<BTreeSet<_>>();
    let mut visited = BTreeSet::new();
    while let Some((package_id, path, abi)) = pending.pop_first() {
        if !visited.insert((package_id.clone(), path.clone(), abi.clone())) {
            continue;
        }
        let Some(fields) = package_type_records.get(&(package_id.clone(), path.clone())) else {
            continue;
        };
        closure.insert((package_id.clone(), path), abi.clone());
        let mut nested = Vec::new();
        for field in fields.values() {
            collect_package_symbols(field, &mut nested);
        }
        for symbol in nested {
            let PackageRefIr::PackageId {
                package_id: nested_owner,
            } = &symbol.package
            else {
                continue;
            };
            if nested_owner != &package_id
                || symbol
                    .abi_expectation
                    .as_deref()
                    .is_some_and(|nested_abi| nested_abi != abi)
                || !package_type_records
                    .contains_key(&(package_id.clone(), symbol.symbol_path.clone()))
            {
                continue;
            }
            let exact_rows = external_refs
                .package_symbols
                .iter()
                .filter(|candidate| {
                    candidate.package
                        == (PackageRefIr::PackageId {
                            package_id: package_id.clone(),
                        })
                        && candidate.symbol_path == symbol.symbol_path
                        && nonempty_abi(candidate) == Some(abi.as_str())
                })
                .count();
            if exact_rows == 0 {
                external_refs.package_symbols.push(PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: package_id.clone(),
                    },
                    symbol_path: symbol.symbol_path.clone(),
                    abi_expectation: Some(abi.clone()),
                });
            } else if exact_rows != 1 {
                continue;
            }
            pending.insert((package_id.clone(), symbol.symbol_path, abi.clone()));
        }
    }

    let record_keys = package_type_records
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    for ((package_id, path), abi) in closure {
        let Some(fields) = package_type_records.get_mut(&(package_id.clone(), path)) else {
            continue;
        };
        for field in fields.values_mut() {
            stamp_same_package_abi(field, &package_id, &abi, &record_keys);
        }
    }
}

fn package_abis(external_refs: &ExternalRefTable) -> BTreeMap<String, BTreeSet<String>> {
    external_refs.package_symbols.iter().fold(
        BTreeMap::<String, BTreeSet<String>>::new(),
        |mut by_owner, symbol| {
            if let (PackageRefIr::PackageId { package_id }, Some(abi)) =
                (&symbol.package, nonempty_abi(symbol))
            {
                by_owner
                    .entry(package_id.clone())
                    .or_default()
                    .insert(abi.to_string());
            }
            by_owner
        },
    )
}

fn collect_package_symbols(ty: &TypeRefIr, symbols: &mut Vec<PackageSymbolRef>) {
    match ty {
        TypeRefIr::Builtin { args, .. } => {
            for argument in args {
                collect_package_symbols(argument, symbols);
            }
        }
        TypeRefIr::PackageSymbol { symbol } => symbols.push(symbol.clone()),
        TypeRefIr::AppliedNominal { base, arguments } => {
            if let skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { symbol } = base {
                symbols.push(symbol.clone());
            }
            for argument in arguments {
                collect_package_symbols(argument, symbols);
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                collect_package_symbols(field, symbols);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_package_symbols(item, symbols);
            }
        }
        TypeRefIr::Nullable { inner } => collect_package_symbols(inner, symbols),
        TypeRefIr::AnyInterface { interface } => {
            for argument in &interface.canonical_type_args {
                collect_package_symbols(argument, symbols);
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                collect_package_symbols(&parameter.ty, symbols);
            }
            collect_package_symbols(return_type, symbols);
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
}

fn stamp_same_package_abi(
    ty: &mut TypeRefIr,
    package_id: &str,
    abi: &str,
    record_keys: &BTreeSet<(String, String)>,
) {
    *ty = skiff_compiler_core::type_ref::map_type_ref(ty.clone(), &mut |ty| match ty {
        TypeRefIr::PackageSymbol { mut symbol } => {
            if symbol.package
                == (PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                })
                && symbol.abi_expectation.is_none()
                && record_keys.contains(&(package_id.to_string(), symbol.symbol_path.clone()))
            {
                symbol.abi_expectation = Some(abi.to_string());
            }
            TypeRefIr::PackageSymbol { symbol }
        }
        TypeRefIr::AppliedNominal {
            mut base,
            arguments,
        } => {
            if let skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { symbol } = &mut base
            {
                if symbol.package
                    == (PackageRefIr::PackageId {
                        package_id: package_id.to_string(),
                    })
                    && symbol.abi_expectation.is_none()
                    && record_keys.contains(&(package_id.to_string(), symbol.symbol_path.clone()))
                {
                    symbol.abi_expectation = Some(abi.to_string());
                }
            }
            TypeRefIr::AppliedNominal { base, arguments }
        }
        other => other,
    });
}

fn nonempty_abi(symbol: &PackageSymbolRef) -> Option<&str> {
    symbol
        .abi_expectation
        .as_deref()
        .filter(|abi| !abi.trim().is_empty())
}

fn package_symbol_sort_key(symbol: &PackageSymbolRef) -> (u8, &str, &str, &str) {
    let (kind, owner) = match &symbol.package {
        PackageRefIr::PackageId { package_id } => (0, package_id.as_str()),
        PackageRefIr::Dependency { dependency_ref } => (1, dependency_ref.as_str()),
    };
    (
        kind,
        owner,
        symbol.symbol_path.as_str(),
        symbol.abi_expectation.as_deref().unwrap_or_default(),
    )
}

#[cfg(test)]
fn project_canonical_package_symbols(
    external_refs: &ExternalRefTable,
    package_type_records: &BTreeMap<(String, String), BTreeMap<String, TypeRefIr>>,
) -> ExternalRefTable {
    project_package_type_authority(external_refs, package_type_records).external_refs
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABI: &str = "sha256:exact-http-abi";
    const HANDLE: &str = "std.http.HttpClientStreamHandle";
    const REQUEST: &str = "std.http.HttpClientRequest";

    #[test]
    fn canonical_projection_is_order_independent_and_record_exact() {
        let relative = dependency_symbol(HANDLE, Some(ABI));
        let anchor = package_symbol("skiff.run/std", REQUEST, ABI);
        let records = exact_records();
        let first = project_canonical_package_symbols(
            &external_refs(vec![relative.clone(), anchor.clone()]),
            &records,
        );
        let second =
            project_canonical_package_symbols(&external_refs(vec![anchor, relative]), &records);

        assert_eq!(first, second);
        assert_eq!(canonical_handle_owners(&first), vec!["skiff.run/std"]);

        let mut drifted = records;
        drifted.insert(
            ("std".to_string(), HANDLE.to_string()),
            BTreeMap::from([("body".to_string(), TypeRefIr::builtin("string"))]),
        );
        let projected = project_canonical_package_symbols(
            &external_refs(vec![
                dependency_symbol(HANDLE, Some(ABI)),
                package_symbol("skiff.run/std", REQUEST, ABI),
            ]),
            &drifted,
        );
        assert!(canonical_handle_owners(&projected).is_empty());
    }

    #[test]
    fn canonical_projection_rejects_ambiguous_or_drifted_abi_anchors() {
        let ambiguous = project_canonical_package_symbols(
            &external_refs(vec![
                dependency_symbol(HANDLE, Some(ABI)),
                package_symbol("skiff.run/std", REQUEST, ABI),
                package_symbol("skiff.run/other", "other.Anchor", ABI),
            ]),
            &exact_records(),
        );
        assert!(canonical_handle_owners(&ambiguous).is_empty());

        let drifted = project_canonical_package_symbols(
            &external_refs(vec![
                dependency_symbol(HANDLE, Some("sha256:drift")),
                package_symbol("skiff.run/std", REQUEST, ABI),
            ]),
            &exact_records(),
        );
        assert!(canonical_handle_owners(&drifted).is_empty());
    }

    #[test]
    fn canonical_projection_rejects_missing_abi_owner_or_records() {
        for symbols in [
            vec![
                dependency_symbol(HANDLE, None),
                package_symbol("skiff.run/std", REQUEST, ABI),
            ],
            vec![dependency_symbol(HANDLE, Some(ABI))],
        ] {
            let projected =
                project_canonical_package_symbols(&external_refs(symbols), &exact_records());
            assert!(canonical_handle_owners(&projected).is_empty());
        }

        let mut missing_package_record = exact_records();
        missing_package_record.remove(&("skiff.run/std".to_string(), HANDLE.to_string()));
        let projected = project_canonical_package_symbols(
            &external_refs(vec![
                dependency_symbol(HANDLE, Some(ABI)),
                package_symbol("skiff.run/std", REQUEST, ABI),
            ]),
            &missing_package_record,
        );
        assert!(canonical_handle_owners(&projected).is_empty());
    }

    #[test]
    fn recursive_record_closure_stamps_nested_same_package_abi() {
        let records = nested_records(package_type("skiff.run/std", "std.http.HttpHeader", None));
        let projection = project_package_type_authority(
            &external_refs(vec![package_symbol("skiff.run/std", REQUEST, ABI)]),
            &records,
        );
        assert_eq!(
            canonical_owners_for_path(&projection.external_refs, "std.http.HttpHeader", ABI),
            vec!["skiff.run/std"]
        );
        let headers = &projection.package_type_records
            [&("skiff.run/std".to_string(), REQUEST.to_string())]["headers"];
        let TypeRefIr::Builtin { args, .. } = headers else {
            panic!("headers remains Array<T>")
        };
        let TypeRefIr::PackageSymbol { symbol } = &args[0] else {
            panic!("headers item remains a package symbol")
        };
        assert_eq!(symbol.abi_expectation.as_deref(), Some(ABI));
    }

    #[test]
    fn recursive_record_closure_does_not_lend_abi_to_foreign_missing_or_drifted_types() {
        for (nested, include_record) in [
            (
                package_type("skiff.run/foreign", "foreign.HttpHeader", None),
                true,
            ),
            (
                package_type("skiff.run/std", "std.http.HttpHeader", None),
                false,
            ),
            (
                package_type("skiff.run/std", "std.http.HttpHeader", Some("sha256:drift")),
                true,
            ),
        ] {
            let mut records = nested_records(nested);
            if !include_record {
                records.remove(&(
                    "skiff.run/std".to_string(),
                    "std.http.HttpHeader".to_string(),
                ));
            }
            let projection = project_package_type_authority(
                &external_refs(vec![package_symbol("skiff.run/std", REQUEST, ABI)]),
                &records,
            );
            assert!(canonical_owners_for_path(
                &projection.external_refs,
                "std.http.HttpHeader",
                ABI
            )
            .is_empty());
        }

        let projection = project_package_type_authority(
            &external_refs(vec![
                package_symbol("skiff.run/std", REQUEST, ABI),
                package_symbol("skiff.run/std", "std.other.Anchor", "sha256:other"),
            ]),
            &nested_records(package_type("skiff.run/std", "std.http.HttpHeader", None)),
        );
        assert!(
            canonical_owners_for_path(&projection.external_refs, "std.http.HttpHeader", ABI)
                .is_empty()
        );
    }

    #[test]
    fn recursive_record_cycle_terminates_without_lending_foreign_abi() {
        let mut records =
            nested_records(package_type("skiff.run/std", "std.http.HttpHeader", None));
        records
            .get_mut(&(
                "skiff.run/std".to_string(),
                "std.http.HttpHeader".to_string(),
            ))
            .expect("nested header record exists")
            .extend([
                (
                    "request".to_string(),
                    package_type("skiff.run/std", REQUEST, None),
                ),
                (
                    "foreign".to_string(),
                    package_type("skiff.run/foreign", "foreign.Header", None),
                ),
            ]);
        let projection = project_package_type_authority(
            &external_refs(vec![package_symbol("skiff.run/std", REQUEST, ABI)]),
            &records,
        );

        assert_eq!(
            canonical_owners_for_path(&projection.external_refs, "std.http.HttpHeader", ABI),
            vec!["skiff.run/std"]
        );
        assert!(projection
            .external_refs
            .package_symbols
            .iter()
            .all(|symbol| symbol.symbol_path != "foreign.Header"));
    }

    fn exact_records() -> BTreeMap<(String, String), BTreeMap<String, TypeRefIr>> {
        let handle = BTreeMap::from([("body".to_string(), TypeRefIr::builtin("bytes"))]);
        BTreeMap::from([
            (("std".to_string(), HANDLE.to_string()), handle.clone()),
            (("skiff.run/std".to_string(), HANDLE.to_string()), handle),
        ])
    }

    fn nested_records(
        nested: TypeRefIr,
    ) -> BTreeMap<(String, String), BTreeMap<String, TypeRefIr>> {
        BTreeMap::from([
            (
                ("skiff.run/std".to_string(), REQUEST.to_string()),
                BTreeMap::from([(
                    "headers".to_string(),
                    TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![nested],
                    },
                )]),
            ),
            (
                (
                    "skiff.run/std".to_string(),
                    "std.http.HttpHeader".to_string(),
                ),
                BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
            ),
        ])
    }

    fn package_type(package_id: &str, path: &str, abi: Option<&str>) -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol_path: path.to_string(),
                abi_expectation: abi.map(str::to_string),
            },
        }
    }

    fn external_refs(package_symbols: Vec<PackageSymbolRef>) -> ExternalRefTable {
        ExternalRefTable {
            package_symbols,
            ..ExternalRefTable::default()
        }
    }

    fn dependency_symbol(path: &str, abi: Option<&str>) -> PackageSymbolRef {
        PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "std".to_string(),
            },
            symbol_path: path.to_string(),
            abi_expectation: abi.map(str::to_string),
        }
    }

    fn package_symbol(package_id: &str, path: &str, abi: &str) -> PackageSymbolRef {
        PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: path.to_string(),
            abi_expectation: Some(abi.to_string()),
        }
    }

    fn canonical_handle_owners(external_refs: &ExternalRefTable) -> Vec<&str> {
        canonical_owners_for_path(external_refs, HANDLE, ABI)
    }

    fn canonical_owners_for_path<'a>(
        external_refs: &'a ExternalRefTable,
        path: &str,
        abi: &str,
    ) -> Vec<&'a str> {
        external_refs
            .package_symbols
            .iter()
            .filter_map(|symbol| {
                if symbol.symbol_path != path || symbol.abi_expectation.as_deref() != Some(abi) {
                    return None;
                }
                match &symbol.package {
                    PackageRefIr::PackageId { package_id } => Some(package_id.as_str()),
                    PackageRefIr::Dependency { .. } => None,
                }
            })
            .collect()
    }
}
