use std::collections::{BTreeMap, BTreeSet};

pub const PRELUDE_REGISTRY_ID: &str = "skiff.prelude";
pub const RESERVED_ROOT_NAMES: &[&str] = &["service", "std", "connect", "config", "root"];
pub const LANGUAGE_PRIMITIVES: &[&str] = &[
    "string", "number", "integer", "bool", "boolean", "null", "unknown", "void", "never",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBindingShape {
    pub type_params: Vec<String>,
    pub params: Vec<String>,
    pub return_type: String,
}

pub fn validate_root_projection_metadata(
    prelude_roots: &[String],
    root_projections: &BTreeMap<String, BTreeMap<String, String>>,
    source_modules: &[String],
) -> Result<(), String> {
    let prelude_roots = prelude_roots.iter().collect::<BTreeSet<_>>();
    let source_modules = source_modules.iter().collect::<BTreeSet<_>>();
    for (root, projections) in root_projections {
        if !prelude_roots.contains(root) {
            return Err(format!(
                "rootProjections includes {root}, but {root} is not declared in prelude.roots"
            ));
        }
        for (name, module) in projections {
            if name.trim().is_empty() || module.trim().is_empty() {
                return Err("rootProjections entries must be non-empty".to_string());
            }
            if !source_modules.contains(module) {
                return Err(format!(
                    "rootProjections.{root}.{name} points to {module}, but no standard_library source module provides it"
                ));
            }
        }
    }
    Ok(())
}

pub fn is_prelude_canonical_type(name: &str) -> bool {
    matches!(name, "Json" | "JsonObject")
}

pub fn primitive_type_symbols() -> BTreeMap<String, String> {
    LANGUAGE_PRIMITIVES
        .iter()
        .map(|name| {
            let symbol = if *name == "boolean" { "bool" } else { name };
            ((*name).to_string(), symbol.to_string())
        })
        .collect()
}

pub fn is_language_builtin_type_name(name: &str) -> bool {
    LANGUAGE_PRIMITIVES
        .iter()
        .any(|primitive| primitive == &name)
}

pub fn qualified_prelude_type(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("std.")?;
    let (module, bare) = rest.rsplit_once('.')?;
    if module.contains('.') || bare.is_empty() {
        return None;
    }
    Some((&name[..name.len() - bare.len() - 1], bare))
}

pub fn config_prelude_type(name: &str) -> Option<(&str, &str)> {
    let bare = name.strip_prefix("config.")?;
    if bare.is_empty() || bare.contains('.') {
        return None;
    }
    Some(("config", bare))
}

pub fn module_symbol_root(package_id: &str, module_path: &str) -> String {
    if package_id == PRELUDE_REGISTRY_ID {
        if module_path == "config" {
            return "config".to_string();
        }
        if module_path.contains('.') {
            return module_path.to_string();
        }
        if module_path.starts_with("std.") {
            return module_path.to_string();
        }
        return format!("std.{module_path}");
    }
    if module_path == package_id || module_path.starts_with(&format!("{package_id}.")) {
        module_path.to_string()
    } else {
        format!("{package_id}.{module_path}")
    }
}

pub fn compiler_owned_type_symbol(name: &str) -> Option<&'static str> {
    match name {
        "Array" => Some("std.collection.Array"),
        "Date" => Some("Date"),
        "Map" => Some("std.collection.Map"),
        "Stream" => Some("std.stream.Stream"),
        "bytes" => Some("std.bytes.bytes"),
        "Json" => Some("Json"),
        "JsonObject" => Some("JsonObject"),
        "WebSocketConnectResult" => Some("std.websocket.WebSocketConnectResult"),
        "ConnectionMessage" => Some("std.websocket.ConnectionMessage"),
        "TextConnectionMessage" => Some("std.websocket.TextConnectionMessage"),
        "BinaryConnectionMessage" => Some("std.websocket.BinaryConnectionMessage"),
        "ActorRef" => Some("ActorRef"),
        _ => None,
    }
}

pub fn schema_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "integer"
            | "number"
            | "bool"
            | "boolean"
            | "null"
            | "unknown"
            | "void"
            | "never"
            | "Date"
    )
}

pub fn validate_package_api_public_path(
    public_path: &str,
    package_id: &str,
    violations: &mut Vec<String>,
) {
    if !public_path.is_empty() && !crate::export_config::is_valid_dotted_module_path(public_path) {
        violations.push(format!(
            "api key {public_path} must be a valid dotted public path or empty string"
        ));
    }
    if !public_path.is_empty()
        && (public_path == package_id || public_path.starts_with(&format!("{package_id}.")))
    {
        violations.push(format!(
            "api key {public_path} must not contain package or service id {package_id}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        compiler_owned_type_symbol, config_prelude_type, module_symbol_root,
        qualified_prelude_type, validate_package_api_public_path,
        validate_root_projection_metadata, PRELUDE_REGISTRY_ID,
    };

    #[test]
    fn metadata_validation_requires_declared_prelude_root() {
        let error = validate_root_projection_metadata(
            &[String::from("config")],
            &BTreeMap::from([(
                String::from("std"),
                BTreeMap::from([(String::from("string"), String::from("std.string"))]),
            )]),
            &[String::from("std.string")],
        )
        .unwrap_err();

        assert_eq!(
            error,
            "rootProjections includes std, but std is not declared in prelude.roots"
        );
    }

    #[test]
    fn metadata_validation_requires_backing_source_module() {
        let error = validate_root_projection_metadata(
            &[String::from("std")],
            &BTreeMap::from([(
                String::from("std"),
                BTreeMap::from([(String::from("string"), String::from("std.string"))]),
            )]),
            &[String::from("std.number")],
        )
        .unwrap_err();

        assert_eq!(
            error,
            "rootProjections.std.string points to std.string, but no standard_library source module provides it"
        );
    }

    #[test]
    fn module_symbol_root_maps_prelude_modules() {
        assert_eq!(module_symbol_root(PRELUDE_REGISTRY_ID, "config"), "config");
        assert_eq!(
            module_symbol_root(PRELUDE_REGISTRY_ID, "collection"),
            "std.collection"
        );
        assert_eq!(
            module_symbol_root(PRELUDE_REGISTRY_ID, "std.string"),
            "std.string"
        );
        assert_eq!(
            module_symbol_root("example.com/pkg", "api"),
            "example.com/pkg.api"
        );
    }

    #[test]
    fn prelude_type_helpers_parse_supported_forms() {
        assert_eq!(
            qualified_prelude_type("std.collection.Array"),
            Some(("std.collection", "Array"))
        );
        assert_eq!(qualified_prelude_type("std.collection.deep.Array"), None);
        assert_eq!(
            config_prelude_type("config.DecodeError"),
            Some(("config", "DecodeError"))
        );
        assert_eq!(config_prelude_type("config.deep.Type"), None);
        assert_eq!(compiler_owned_type_symbol("JsonObject"), Some("JsonObject"));
    }

    #[test]
    fn package_api_public_path_validation_is_context_free() {
        let mut violations = Vec::new();
        validate_package_api_public_path("example.com/pkg.api", "example.com/pkg", &mut violations);
        validate_package_api_public_path("bad-path", "example.com/pkg", &mut violations);

        assert_eq!(
            violations,
            vec![
                "api key example.com/pkg.api must be a valid dotted public path or empty string",
                "api key example.com/pkg.api must not contain package or service id example.com/pkg",
                "api key bad-path must be a valid dotted public path or empty string",
            ]
        );
    }
}
