use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::STD_NATIVE_SIGNATURES;

use crate::shared::type_syntax::generic_parts;

use super::{
    default_prelude_dir, default_std_dir, native_type_expr_def_normalized_name, prelude_registry,
    prelude_schema_identity, shared_native_alias_target, NativeBindingShape, PreludeRegistry,
};

#[test]
fn compiler_owned_schema_stable_types_have_canonical_symbols() {
    let registry = PreludeRegistry::empty();

    for name in [
        "Array",
        "Date",
        "Map",
        "bytes",
        "Json",
        "JsonObject",
        "WebSocketConnectResult",
        "ConnectionMessage",
        "TextConnectionMessage",
        "BinaryConnectionMessage",
    ] {
        assert_ne!(registry.type_symbol(name), "std.unknown");
    }

    let registry = prelude_registry();
    for name in [
        "LlmRole",
        "LlmMessage",
        "LlmChatRequest",
        "LlmUsage",
        "LlmChatResponse",
    ] {
        assert_ne!(registry.type_symbol(name), "std.unknown");
    }
}

#[test]
fn builtin_type_helper_includes_prelude_date_and_language_aliases() {
    let registry = prelude_registry();

    assert!(registry.is_prelude_type_name("Date"));
    assert!(registry.is_schema_stable_type("Date"));
    assert_eq!(registry.known_type_symbol("Date").as_deref(), Some("Date"));
    assert!(registry.is_prelude_type_name("Duration"));
    assert!(registry.is_schema_stable_type("Duration"));
    assert_eq!(
        registry.known_type_symbol("Duration").as_deref(),
        Some("std.time.Duration")
    );
    assert!(super::is_builtin_type_name("Date"));
    assert!(super::is_builtin_type_name("Duration"));
    assert!(super::is_builtin_type_name("boolean"));
    assert!(!super::is_builtin_type_name("Result"));
}

#[test]
fn std_exported_interfaces_have_known_symbols_without_schema_stability() {
    let registry = prelude_registry();

    assert_eq!(
        registry.known_type_symbol("std.actor.Actor").as_deref(),
        Some("std.actor.Actor")
    );
    assert_eq!(
        registry.known_type_symbol("Actor").as_deref(),
        Some("std.actor.Actor")
    );
    assert!(!registry.is_schema_stable_type("Actor"));
}

#[test]
fn duplicate_std_type_names_are_resolved_by_qualified_symbol() {
    let registry = prelude_registry();

    for name in [
        "config.DecodeError",
        "std.bytes.DecodeError",
        "std.db.DecodeError",
        "std.json.DecodeError",
        "std.number.DecodeError",
        "std.time.DecodeError",
    ] {
        assert_eq!(registry.known_type_symbol(name).as_deref(), Some(name));
        assert!(registry.type_decl(name).is_some(), "{name} should resolve");
        assert!(
            registry.is_schema_stable_type(name),
            "{name} should be schema-stable"
        );
    }
    assert!(registry.root_projection_roots("std").contains("number"));
}

#[test]
fn split_registry_identities_match_builtin_registry_accessors() {
    let registry =
        PreludeRegistry::try_from_split_dirs(&default_prelude_dir(), &default_std_dir()).unwrap();

    assert_eq!(registry.schema_identity(), prelude_schema_identity());
    assert_eq!(
        registry.native_identity(),
        prelude_registry().native_identity()
    );
}

#[test]
fn std_native_aliases_are_derived_from_shared_signatures() {
    let registry = prelude_registry();
    let shared_aliases = shared_alias_targets();
    let installed_aliases = registry
        .native_bindings
        .keys()
        .filter_map(|symbol| {
            shared_native_alias_target(symbol).map(|target| (symbol.clone(), target))
        })
        .collect::<BTreeMap<_, _>>();

    assert_eq!(
        installed_aliases, shared_aliases,
        "compiler native alias bindings should match the shared native signature aliases"
    );
    for (alias, target) in shared_aliases {
        assert_eq!(shared_native_alias_target(&alias), Some(target));
        assert!(
            registry.is_native_symbol(&alias),
            "shared native alias {alias} should be installed in compiler registry"
        );
        assert!(
            registry.is_native_symbol(target),
            "shared native alias canonical target {target} should be installed in compiler registry"
        );
    }
}

#[test]
fn compiler_std_native_binding_keys_are_derived_from_shared_signatures() {
    let shared_aliases = shared_alias_targets();
    for signature in STD_NATIVE_SIGNATURES {
        assert_eq!(
            super::shared_native_binding_key(signature.target),
            Some(signature.binding_key),
            "compiler binding key lookup for {} drifted from shared native signature",
            signature.target
        );

        for alias in signature.aliases {
            let expected = if shared_aliases.contains_key(*alias) {
                Some(signature.binding_key)
            } else {
                None
            };
            assert_eq!(
                super::shared_native_binding_key(alias),
                expected,
                "compiler binding key lookup for alias {alias} drifted from shared native signature"
            );
        }
    }
}

#[test]
fn compiler_std_native_declarations_match_shared_signatures() {
    let registry = prelude_registry();
    let shared = STD_NATIVE_SIGNATURES
        .iter()
        .map(|signature| (signature.target.to_string(), shared_native_shape(signature)))
        .collect::<BTreeMap<_, _>>();
    let declared = canonical_shared_declared_native_bindings(registry);

    assert_eq!(
        declared.keys().collect::<Vec<_>>(),
        shared.keys().collect::<Vec<_>>(),
        "compiler declared native bindings should directly expose every shared canonical target"
    );

    for (target, expected) in shared {
        let actual = declared.get(&target).unwrap_or_else(|| {
            panic!("compiler declaration is missing direct shared native target {target}")
        });
        assert_eq!(
            actual, &expected,
            "compiler declaration for {target} drifted from shared native signature"
        );
    }

    let shared_declared_names = STD_NATIVE_SIGNATURES
        .iter()
        .flat_map(|signature| {
            std::iter::once(signature.target.to_string())
                .chain(signature.aliases.iter().map(|alias| (*alias).to_string()))
        })
        .collect::<BTreeSet<_>>();
    let compiler_shared_names = registry
        .raw_declared_native_bindings
        .keys()
        .filter(|symbol| !runtime_builtin_only_native_declaration(symbol))
        .cloned()
        .collect::<BTreeSet<_>>();

    let compiler_only = compiler_shared_names
        .difference(&shared_declared_names)
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        compiler_only.is_empty(),
        "compiler std/prelude native declarations should be either shared native signatures or explicit runtime builtin-only exclusions; missing shared entries for {compiler_only:?}"
    );
}

#[test]
fn declared_std_native_aliases_match_canonical_shapes() {
    let registry = prelude_registry();

    for signature in STD_NATIVE_SIGNATURES {
        let canonical = registry
            .declared_native_bindings
            .get(signature.target)
            .unwrap_or_else(|| {
                panic!(
                    "compiler declaration is missing direct shared native target {}",
                    signature.target
                )
            });
        let expected = normalize_native_binding_shape(&canonical.shape);
        let raw = canonical_declared_native_binding(registry, signature);
        assert_eq!(
            normalize_native_binding_shape(&raw.shape),
            expected,
            "direct compiler declaration for canonical target {} drifted from registered canonical binding",
            signature.target
        );

        for alias in signature.aliases {
            if let Some(alias_binding) = registry.declared_native_bindings.get(*alias) {
                assert_eq!(
                    normalize_native_binding_shape(&alias_binding.shape),
                    expected,
                    "compiler declaration for alias {alias} drifted from canonical target {}",
                    signature.target
                );
            }
        }
    }
}

fn canonical_declared_native_binding<'a>(
    registry: &'a PreludeRegistry,
    signature: &skiff_artifact_model::NativeSignatureDef,
) -> &'a super::NativeBinding {
    if let Some(raw) = registry.raw_declared_native_bindings.get(signature.target) {
        return raw;
    }

    for alias in signature.aliases {
        if let Some(raw) = registry.raw_declared_native_bindings.get(*alias) {
            assert!(
                canonical_backfill_alias_allowed(signature.target, alias),
                "shared canonical target {} must be declared directly in std/prelude sources; alias {alias} cannot replace the canonical declaration",
                signature.target
            );
            return raw;
        }
    }

    panic!(
        "shared canonical target {} is neither declared directly nor via an allowed prelude impl alias",
        signature.target
    );
}

fn canonical_backfill_alias_allowed(target: &str, alias: &str) -> bool {
    matches!(alias.split('.').next(), Some("number" | "string" | "bytes"))
        && target == format!("std.{alias}")
}

fn runtime_builtin_only_native_declaration(symbol: &str) -> bool {
    matches!(
        symbol,
        "string.length"
            | "string.contains"
            | "string.concat"
            | "string.lowercase"
            | "string.replaceAll"
            | "string.startsWith"
            | "string.endsWith"
            | "Array.length"
            | "Array.push"
            | "Array.set"
            | "Array.pop"
            | "Array.clone"
            | "Array.map"
            | "Array.filter"
            | "Map.length"
            | "Map.get"
            | "Map.has"
            | "Map.set"
            | "Map.delete"
            | "Map.keys"
            | "Map.clone"
            | "number.floor"
            | "number.ceil"
            | "number.round"
            | "bytes.length"
            | "bytes.toBase64"
            | "bytes.toHex"
            | "bytes.toUtf8String"
    )
}

fn shared_alias_targets() -> BTreeMap<String, &'static str> {
    STD_NATIVE_SIGNATURES
        .iter()
        .flat_map(|signature| {
            signature
                .aliases
                .iter()
                .map(move |alias| ((*alias).to_string(), signature.target))
        })
        .collect()
}

fn canonical_shared_declared_native_bindings(
    registry: &PreludeRegistry,
) -> BTreeMap<String, NativeBindingShape> {
    STD_NATIVE_SIGNATURES
        .iter()
        .filter_map(|signature| {
            registry
                .declared_native_bindings
                .get(signature.target)
                .map(|binding| {
                    (
                        signature.target.to_string(),
                        normalize_native_binding_shape(&binding.shape),
                    )
                })
        })
        .collect()
}

fn shared_native_shape(signature: &skiff_artifact_model::NativeSignatureDef) -> NativeBindingShape {
    NativeBindingShape {
        type_params: (0..signature.type_param_count)
            .map(|index| format!("T{index}"))
            .collect(),
        params: signature
            .params
            .iter()
            .map(native_type_expr_def_normalized_name)
            .collect(),
        return_type: native_type_expr_def_normalized_name(&signature.return_type),
    }
}

fn normalize_native_binding_shape(shape: &NativeBindingShape) -> NativeBindingShape {
    let type_param_map = shape
        .type_params
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), format!("T{index}")))
        .collect::<BTreeMap<_, _>>();

    NativeBindingShape {
        type_params: (0..shape.type_params.len())
            .map(|index| format!("T{index}"))
            .collect(),
        params: shape
            .params
            .iter()
            .map(|param| normalize_compiler_native_type_name(param, &type_param_map))
            .collect(),
        return_type: normalize_compiler_native_type_name(&shape.return_type, &type_param_map),
    }
}

fn normalize_compiler_native_type_name(
    name: &str,
    type_param_map: &BTreeMap<&str, String>,
) -> String {
    let name = name.trim();
    if let Some(inner) = name.strip_suffix('?') {
        return format!(
            "{}?",
            normalize_compiler_native_type_name(inner, type_param_map)
        );
    }
    if let Some(mapped) = type_param_map.get(name) {
        return mapped.clone();
    }
    if let Some(parts) = generic_parts(name) {
        let root = parts.root.trim().to_string();
        let args = parts
            .args
            .into_iter()
            .map(|arg| normalize_compiler_native_type_name(arg, type_param_map))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{root}<{args}>");
    }
    name.to_string()
}
