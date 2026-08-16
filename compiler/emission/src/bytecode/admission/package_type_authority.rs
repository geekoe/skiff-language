use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    NominalTypeRefBaseIr, PackageRefIr, PackageSymbolRef, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::type_ref::map_type_ref;
use skiff_compiler_lowering::mir::{
    MirForInBinding, MirFunction, MirStmtKind, MirUnit, MirWritablePathSegment, MirWritableRoot,
};

/// Consumes only the exact same-path canonical rows projected by MIR. The
/// dependency alias itself is never interpreted here; missing ABI, duplicate
/// rows, or owner disagreement leaves the type relative so ordinary admission
/// rejects it.
#[derive(Debug)]
pub(super) struct PackageTypeAuthorityError {
    pub unit_index: usize,
    pub detail: String,
}

pub(super) fn normalize_package_type_authorities(
    units: &[MirUnit],
) -> Result<Vec<MirUnit>, PackageTypeAuthorityError> {
    units
        .iter()
        .cloned()
        .enumerate()
        .map(|(unit_index, mut unit)| {
            let authority = PackageTypeAuthority::from_unit(&unit);
            normalize_unit(&mut unit, &authority);
            project_recursive_record_closure(&mut unit)
                .map_err(|detail| PackageTypeAuthorityError { unit_index, detail })?;
            let authority = PackageTypeAuthority::from_unit(&unit);
            normalize_unit(&mut unit, &authority);
            Ok(unit)
        })
        .collect()
}

#[derive(Default)]
struct PackageTypeAuthority {
    owners: BTreeMap<(String, String, String), String>,
    canonical_abis: BTreeMap<(String, String), String>,
}

impl PackageTypeAuthority {
    fn from_unit(unit: &MirUnit) -> Self {
        let mut relative_counts = BTreeMap::<(String, String, String), usize>::new();
        let mut canonical = BTreeMap::<(String, String), Vec<&PackageSymbolRef>>::new();
        for symbol in &unit.external_refs.package_symbols {
            let Some(abi) = nonempty_abi(symbol) else {
                continue;
            };
            match &symbol.package {
                PackageRefIr::Dependency { dependency_ref } => {
                    *relative_counts
                        .entry((
                            dependency_ref.clone(),
                            symbol.symbol_path.clone(),
                            abi.to_string(),
                        ))
                        .or_default() += 1;
                }
                PackageRefIr::PackageId { .. } => canonical
                    .entry((symbol.symbol_path.clone(), abi.to_string()))
                    .or_default()
                    .push(symbol),
            }
        }

        let owners = relative_counts
            .into_iter()
            .filter_map(|(key, count)| {
                if count != 1 {
                    return None;
                }
                let (_, path, abi) = &key;
                let [candidate] = canonical.get(&(path.clone(), abi.clone()))?.as_slice() else {
                    return None;
                };
                let PackageRefIr::PackageId { package_id } = &candidate.package else {
                    unreachable!("canonical index contains only PackageId rows")
                };
                let (dependency_ref, _, _) = &key;
                let alias_record = unit
                    .package_type_records
                    .get(&(dependency_ref.clone(), path.clone()))?;
                let package_record = unit
                    .package_type_records
                    .get(&(package_id.clone(), path.clone()))?;
                if alias_record != package_record {
                    return None;
                }
                Some((key, package_id.clone()))
            })
            .collect();
        let mut canonical_counts = BTreeMap::<(String, String, String), usize>::new();
        let mut package_abis = BTreeMap::<String, BTreeSet<String>>::new();
        for symbol in &unit.external_refs.package_symbols {
            let (PackageRefIr::PackageId { package_id }, Some(abi)) =
                (&symbol.package, nonempty_abi(symbol))
            else {
                continue;
            };
            *canonical_counts
                .entry((
                    package_id.clone(),
                    symbol.symbol_path.clone(),
                    abi.to_string(),
                ))
                .or_default() += 1;
            package_abis
                .entry(package_id.clone())
                .or_default()
                .insert(abi.to_string());
        }
        let canonical_abis = canonical_counts
            .into_iter()
            .filter_map(|((package_id, path, abi), count)| {
                (count == 1
                    && package_abis.get(&package_id) == Some(&BTreeSet::from([abi.clone()]))
                    && unit
                        .package_type_records
                        .contains_key(&(package_id.clone(), path.clone())))
                .then_some(((package_id, path), abi))
            })
            .collect();
        Self {
            owners,
            canonical_abis,
        }
    }

    fn normalize(&self, ty: TypeRefIr) -> TypeRefIr {
        map_type_ref(ty, &mut |ty| match ty {
            TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol {
                symbol: self.normalize_symbol(symbol),
            },
            TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
                base: match base {
                    NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                        NominalTypeRefBaseIr::PackageSymbol {
                            symbol: self.normalize_symbol(symbol),
                        }
                    }
                    other => other,
                },
                arguments,
            },
            other => other,
        })
    }

    fn normalize_symbol(&self, mut symbol: PackageSymbolRef) -> PackageSymbolRef {
        match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => {
                let Some(abi) = nonempty_abi(&symbol) else {
                    return symbol;
                };
                let key = (
                    dependency_ref.clone(),
                    symbol.symbol_path.clone(),
                    abi.to_string(),
                );
                let Some(package_id) = self.owners.get(&key) else {
                    return symbol;
                };
                symbol.package = PackageRefIr::PackageId {
                    package_id: package_id.clone(),
                };
            }
            PackageRefIr::PackageId { package_id } if symbol.abi_expectation.is_none() => {
                let Some(abi) = self
                    .canonical_abis
                    .get(&(package_id.clone(), symbol.symbol_path.clone()))
                else {
                    return symbol;
                };
                symbol.abi_expectation = Some(abi.clone());
            }
            PackageRefIr::PackageId { .. } => {}
        }
        symbol
    }
}

fn project_recursive_record_closure(unit: &mut MirUnit) -> Result<(), String> {
    let package_abis = unit.external_refs.package_symbols.iter().fold(
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
    );
    let mut exact_counts = BTreeMap::<(String, String, String), usize>::new();
    for symbol in &unit.external_refs.package_symbols {
        let (PackageRefIr::PackageId { package_id }, Some(abi)) =
            (&symbol.package, nonempty_abi(symbol))
        else {
            continue;
        };
        *exact_counts
            .entry((
                package_id.clone(),
                symbol.symbol_path.clone(),
                abi.to_string(),
            ))
            .or_default() += 1;
    }

    let mut pending = BTreeSet::new();
    for ((package_id, path, abi), count) in &exact_counts {
        if !unit
            .package_type_records
            .contains_key(&(package_id.clone(), path.clone()))
        {
            continue;
        }
        if *count != 1 {
            return Err(format!(
                "canonical package record `{package_id}/{path}` has {count} exact external rows"
            ));
        }
        let Some(abis) = package_abis.get(package_id) else {
            continue;
        };
        if abis.len() != 1 || !abis.contains(abi) {
            return Err(format!(
                "canonical package `{package_id}` has ambiguous ABI authority"
            ));
        }
        pending.insert((package_id.clone(), path.clone(), abi.clone()));
    }

    let mut visited = BTreeSet::new();
    let mut additions = BTreeMap::<(String, String, String), PackageSymbolRef>::new();
    while let Some((package_id, path, abi)) = pending.pop_first() {
        if !visited.insert((package_id.clone(), path.clone(), abi.clone())) {
            continue;
        }
        let Some(fields) = unit
            .package_type_records
            .get(&(package_id.clone(), path.clone()))
        else {
            return Err(format!(
                "canonical package record `{package_id}/{path}` is missing"
            ));
        };
        let mut nested = Vec::new();
        for field in fields.values() {
            collect_package_symbols(field, &mut nested);
        }
        for symbol in nested {
            let PackageRefIr::PackageId {
                package_id: nested_owner,
            } = &symbol.package
            else {
                return Err(format!(
                    "canonical package record `{package_id}/{path}` retains a dependency alias"
                ));
            };
            if nested_owner != &package_id {
                if nonempty_abi(&symbol).is_none() {
                    return Err(format!(
                        "canonical package record `{package_id}/{path}` cannot lend ABI authority to foreign owner `{nested_owner}`"
                    ));
                }
                continue;
            }
            if symbol
                .abi_expectation
                .as_deref()
                .is_some_and(|nested_abi| nested_abi != abi)
            {
                return Err(format!(
                    "canonical package record `{package_id}/{}` has ABI drift",
                    symbol.symbol_path
                ));
            }
            if !unit
                .package_type_records
                .contains_key(&(package_id.clone(), symbol.symbol_path.clone()))
            {
                return Err(format!(
                    "nested canonical package record `{package_id}/{}` is missing",
                    symbol.symbol_path
                ));
            }
            let key = (package_id.clone(), symbol.symbol_path.clone(), abi.clone());
            let count = exact_counts.get(&key).copied().unwrap_or_default();
            if count > 1 {
                return Err(format!(
                    "nested canonical package record `{package_id}/{}` has ambiguous ABI rows",
                    symbol.symbol_path
                ));
            }
            if count == 0 {
                additions.entry(key.clone()).or_insert(PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: package_id.clone(),
                    },
                    symbol_path: symbol.symbol_path.clone(),
                    abi_expectation: Some(abi.clone()),
                });
            }
            pending.insert(key);
        }
    }
    unit.external_refs
        .package_symbols
        .extend(additions.into_values());
    unit.external_refs
        .package_symbols
        .sort_by(|left, right| package_symbol_sort_key(left).cmp(&package_symbol_sort_key(right)));
    Ok(())
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
            if let NominalTypeRefBaseIr::PackageSymbol { symbol } = base {
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

fn normalize_unit(unit: &mut MirUnit, authority: &PackageTypeAuthority) {
    for declaration in &mut unit.type_table {
        normalize_descriptor(&mut declaration.descriptor, authority);
        for implemented in &mut declaration.implements {
            normalize_type(implemented, authority);
        }
    }
    for fields in unit.package_type_records.values_mut() {
        for ty in fields.values_mut() {
            normalize_type(ty, authority);
        }
    }
    for constant in &mut unit.constants {
        normalize_type(&mut constant.ty, authority);
    }
    for function in &mut unit.functions {
        normalize_function(function, authority);
    }
}

fn normalize_function(function: &mut MirFunction, authority: &PackageTypeAuthority) {
    for parameter in &mut function.params {
        normalize_type(&mut parameter.ty, authority);
    }
    normalize_type(&mut function.return_type, authority);
    if let Some(ty) = &mut function.self_type {
        normalize_type(ty, authority);
    }
    if let Some(receiver) = &mut function.receiver {
        normalize_type(&mut receiver.ty, authority);
    }
    for slot in &mut function.slots {
        if let Some(ty) = &mut slot.ty {
            normalize_type(ty, authority);
        }
    }
    for access in function.index_accesses.values_mut() {
        normalize_type(&mut access.receiver_type, authority);
        normalize_type(&mut access.selector_type, authority);
        normalize_type(&mut access.result_type, authority);
    }
    for expression in &mut function.expressions {
        normalize_type(&mut expression.ty, authority);
        if let Some(stream) = &mut expression.stream_result {
            normalize_type(&mut stream.item_type, authority);
        }
        if let Some(direct) = &mut expression.direct_call {
            if let Some(receiver) = &mut direct.concrete_receiver {
                normalize_type(receiver, authority);
            }
        }
    }
    for block in &mut function.blocks {
        for statement in &mut block.statements {
            match &mut statement.kind {
                MirStmtKind::Assign { place, .. } => normalize_place(place, authority),
                MirStmtKind::StreamNext { item_type, .. }
                | MirStmtKind::Throw {
                    payload_type: item_type,
                    ..
                } => normalize_type(item_type, authority),
                MirStmtKind::ForIn { facts, .. } => {
                    normalize_type(&mut facts.iterable_type, authority);
                    match &mut facts.binding {
                        MirForInBinding::Item { ty, .. } => normalize_type(ty, authority),
                        MirForInBinding::MapEntry {
                            key_type,
                            value_type,
                            ..
                        } => {
                            normalize_type(key_type, authority);
                            normalize_type(value_type, authority);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for region in &mut function.regions {
        normalize_type(&mut region.catch_type, authority);
    }
    if let Some(stream) = &mut function.stream_result {
        normalize_type(&mut stream.item_type, authority);
    }
}

fn normalize_place(
    place: &mut skiff_compiler_lowering::mir::MirWritablePlace,
    authority: &PackageTypeAuthority,
) {
    if let MirWritableRoot::ActorSelfField { field_type, .. } = &mut place.root {
        normalize_type(field_type, authority);
    }
    for segment in &mut place.path {
        if let MirWritablePathSegment::Index {
            index_type, access, ..
        } = segment
        {
            normalize_type(index_type, authority);
            normalize_type(&mut access.receiver_type, authority);
            normalize_type(&mut access.selector_type, authority);
            normalize_type(&mut access.result_type, authority);
        }
    }
}

fn normalize_descriptor(descriptor: &mut TypeDescriptorIr, authority: &PackageTypeAuthority) {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for ty in fields.values_mut() {
                normalize_type(ty, authority);
            }
        }
        TypeDescriptorIr::Representation { representation } => {
            normalize_type(representation, authority)
        }
        TypeDescriptorIr::Union { branches } => {
            for branch in branches {
                match branch {
                    skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        normalize_type(nominal_type, authority)
                    }
                    skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type,
                        ..
                    } => normalize_type(payload_type, authority),
                    skiff_artifact_model::NamedUnionBranchIr::Literal { .. } => {}
                }
            }
        }
        TypeDescriptorIr::Alias { target } => normalize_type(target, authority),
        TypeDescriptorIr::Interface => {}
    }
}

fn normalize_type(ty: &mut TypeRefIr, authority: &PackageTypeAuthority) {
    *ty = authority.normalize(ty.clone());
}

fn nonempty_abi(symbol: &PackageSymbolRef) -> Option<&str> {
    symbol
        .abi_expectation
        .as_deref()
        .filter(|abi| !abi.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{ExternalRefTable, FileLinkTargets, SourceMapDto};

    const ABI: &str = "sha256:exact-http-abi";
    const HANDLE: &str = "std.http.HttpClientStreamHandle";
    const HEADER: &str = "std.http.HttpHeader";
    const REQUEST: &str = "std.http.HttpClientRequest";

    #[test]
    fn exact_same_path_abi_and_records_normalize_recursively() {
        let unit = unit(
            vec![
                dependency_symbol(Some(ABI)),
                package_symbol("skiff.run/std", ABI),
            ],
            exact_records(),
        );
        let normalized = PackageTypeAuthority::from_unit(&unit).normalize(TypeRefIr::Nullable {
            inner: Box::new(relative_type(Some(ABI))),
        });
        let TypeRefIr::Nullable { inner } = normalized else {
            panic!("nullable shape must be retained")
        };
        assert_eq!(package_owner(&inner), Some("skiff.run/std"));
    }

    #[test]
    fn forged_canonical_row_with_record_drift_stays_relative() {
        let mut records = exact_records();
        records.insert(
            ("skiff.run/std".to_string(), HANDLE.to_string()),
            BTreeMap::from([("body".to_string(), TypeRefIr::builtin("string"))]),
        );
        let unit = unit(
            vec![
                dependency_symbol(Some(ABI)),
                package_symbol("skiff.run/std", ABI),
            ],
            records,
        );

        let normalized = PackageTypeAuthority::from_unit(&unit).normalize(relative_type(Some(ABI)));
        assert_eq!(package_owner(&normalized), None);
    }

    #[test]
    fn ambiguous_canonical_owners_stay_relative() {
        let unit = unit(
            vec![
                dependency_symbol(Some(ABI)),
                package_symbol("skiff.run/std", ABI),
                package_symbol("skiff.run/other", ABI),
            ],
            exact_records(),
        );

        let normalized = PackageTypeAuthority::from_unit(&unit).normalize(relative_type(Some(ABI)));
        assert_eq!(package_owner(&normalized), None);
    }

    #[test]
    fn missing_abi_owner_or_record_stays_relative() {
        let fixtures = [
            unit(
                vec![
                    dependency_symbol(None),
                    package_symbol("skiff.run/std", ABI),
                ],
                exact_records(),
            ),
            unit(vec![dependency_symbol(Some(ABI))], exact_records()),
            unit(
                vec![
                    dependency_symbol(Some(ABI)),
                    package_symbol("skiff.run/std", ABI),
                ],
                BTreeMap::new(),
            ),
        ];
        for (fixture, ty) in fixtures.into_iter().zip([
            relative_type(None),
            relative_type(Some(ABI)),
            relative_type(Some(ABI)),
        ]) {
            let normalized = PackageTypeAuthority::from_unit(&fixture).normalize(ty);
            assert_eq!(package_owner(&normalized), None);
        }
    }

    #[test]
    fn nested_missing_abi_closes_from_exact_same_package_records() {
        let source = unit(
            vec![package_symbol_at("skiff.run/std", REQUEST, ABI)],
            nested_records(package_type("skiff.run/std", HEADER, None), true),
        );
        let normalized = normalize_package_type_authorities(&[source])
            .expect("exact same-package record closure owns the nested ABI");
        assert!(normalized[0]
            .external_refs
            .package_symbols
            .iter()
            .any(|symbol| {
                symbol.package
                    == (PackageRefIr::PackageId {
                        package_id: "skiff.run/std".to_string(),
                    })
                    && symbol.symbol_path == HEADER
                    && symbol.abi_expectation.as_deref() == Some(ABI)
            }));
        let request = &normalized[0].package_type_records
            [&("skiff.run/std".to_string(), REQUEST.to_string())];
        let headers = &request["headers"];
        let TypeRefIr::Builtin { args, .. } = headers else {
            panic!("headers remains Array<T>")
        };
        assert_eq!(package_symbol_abi(&args[0]), Some(ABI));
    }

    #[test]
    fn nested_closure_rejects_foreign_missing_record_drift_and_ambiguous_abi() {
        for records in [
            nested_records(
                package_type("skiff.run/foreign", "foreign.Header", None),
                true,
            ),
            nested_records(
                package_type("skiff.run/foreign", "foreign.Header", Some("")),
                true,
            ),
            nested_records(package_type("skiff.run/std", HEADER, None), false),
            nested_records(
                package_type("skiff.run/std", HEADER, Some("sha256:drift")),
                true,
            ),
        ] {
            let source = unit(
                vec![package_symbol_at("skiff.run/std", REQUEST, ABI)],
                records,
            );
            assert!(normalize_package_type_authorities(&[source]).is_err());
        }

        let source = unit(
            vec![
                package_symbol_at("skiff.run/std", REQUEST, ABI),
                package_symbol_at("skiff.run/std", "std.other.Anchor", "sha256:other"),
            ],
            nested_records(package_type("skiff.run/std", HEADER, None), true),
        );
        assert!(normalize_package_type_authorities(&[source]).is_err());
    }

    #[test]
    fn nested_cycle_cannot_hide_a_foreign_missing_abi() {
        let mut records = nested_records(package_type("skiff.run/std", HEADER, None), true);
        records
            .get_mut(&("skiff.run/std".to_string(), HEADER.to_string()))
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
        let source = unit(
            vec![package_symbol_at("skiff.run/std", REQUEST, ABI)],
            records,
        );

        assert!(normalize_package_type_authorities(&[source]).is_err());
    }

    fn unit(
        package_symbols: Vec<PackageSymbolRef>,
        package_type_records: BTreeMap<(String, String), BTreeMap<String, TypeRefIr>>,
    ) -> MirUnit {
        MirUnit {
            file_ir_identity: "file:authority".to_string(),
            package_id: "test.package".to_string(),
            module_path: "main".to_string(),
            actor_declarations: Vec::new(),
            external_refs: ExternalRefTable {
                package_symbols,
                ..ExternalRefTable::default()
            },
            source_map: SourceMapDto {
                format: String::new(),
                sources: Vec::new(),
                spans: Vec::new(),
            },
            type_table: Vec::new(),
            package_type_records,
            link_targets: FileLinkTargets::default(),
            remote_interface_refs: Vec::new(),
            constants: Vec::new(),
            functions: Vec::new(),
        }
    }

    fn exact_records() -> BTreeMap<(String, String), BTreeMap<String, TypeRefIr>> {
        let fields = BTreeMap::from([("body".to_string(), TypeRefIr::builtin("bytes"))]);
        BTreeMap::from([
            (("std".to_string(), HANDLE.to_string()), fields.clone()),
            (("skiff.run/std".to_string(), HANDLE.to_string()), fields),
        ])
    }

    fn nested_records(
        nested: TypeRefIr,
        include_nested: bool,
    ) -> BTreeMap<(String, String), BTreeMap<String, TypeRefIr>> {
        let mut records = BTreeMap::from([(
            ("skiff.run/std".to_string(), REQUEST.to_string()),
            BTreeMap::from([(
                "headers".to_string(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![nested],
                },
            )]),
        )]);
        if include_nested {
            records.insert(
                ("skiff.run/std".to_string(), HEADER.to_string()),
                BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
            );
            records.insert(
                (
                    "skiff.run/foreign".to_string(),
                    "foreign.Header".to_string(),
                ),
                BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
            );
        }
        records
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

    fn package_symbol_abi(ty: &TypeRefIr) -> Option<&str> {
        let TypeRefIr::PackageSymbol { symbol } = ty else {
            return None;
        };
        symbol.abi_expectation.as_deref()
    }

    fn dependency_symbol(abi: Option<&str>) -> PackageSymbolRef {
        PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "std".to_string(),
            },
            symbol_path: HANDLE.to_string(),
            abi_expectation: abi.map(str::to_string),
        }
    }

    fn package_symbol(package_id: &str, abi: &str) -> PackageSymbolRef {
        package_symbol_at(package_id, HANDLE, abi)
    }

    fn package_symbol_at(package_id: &str, path: &str, abi: &str) -> PackageSymbolRef {
        PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: path.to_string(),
            abi_expectation: Some(abi.to_string()),
        }
    }

    fn relative_type(abi: Option<&str>) -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: dependency_symbol(abi),
        }
    }

    fn package_owner(ty: &TypeRefIr) -> Option<&str> {
        let TypeRefIr::PackageSymbol { symbol } = ty else {
            return None;
        };
        match &symbol.package {
            PackageRefIr::PackageId { package_id } => Some(package_id.as_str()),
            PackageRefIr::Dependency { .. } => None,
        }
    }
}
