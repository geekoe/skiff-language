use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    LiteralIr, PackageRefIr, PackageSymbolRef, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
};

use crate::{
    package_export_resolver::PackageExportResolver,
    shared::ast::{AliasDecl, SourceFile, TypeDecl, TypeRef},
    shared::id::SKIFF_STD_PUBLICATION_ID,
    shared::prelude_registry::prelude_registry,
    shared::type_syntax::{generic_parts, split_top_level, string_literal},
};
use compiler_input_model::is_standard_package_id;

#[derive(Debug, Clone)]
pub struct ProviderRuntimePolicy {
    pub package_id: Option<String>,
}

impl ProviderRuntimePolicy {
    pub fn service_source(_ast: &SourceFile) -> Self {
        Self { package_id: None }
    }

    pub fn disabled() -> Self {
        Self { package_id: None }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeBindings {
    pub policy: ProviderRuntimePolicy,
    pub module_path: Option<String>,
    pub package_aliases: BTreeMap<String, Vec<String>>,
    pub type_decls: BTreeMap<String, TypeDecl>,
    pub aliases: BTreeMap<String, AliasDecl>,
}

impl RuntimeBindings {
    pub fn new(policy: ProviderRuntimePolicy) -> Self {
        Self {
            policy,
            module_path: None,
            package_aliases: BTreeMap::new(),
            type_decls: BTreeMap::new(),
            aliases: BTreeMap::new(),
        }
    }

    pub fn with_module_path(mut self, module_path: &str) -> Self {
        self.module_path = Some(module_path.to_string());
        self
    }

    pub fn with_source_types(mut self, ast: &SourceFile) -> Self {
        self.type_decls = ast
            .types
            .iter()
            .map(|ty| (ty.name.clone(), ty.clone()))
            .collect();
        self.aliases = ast
            .aliases
            .iter()
            .map(|alias| (alias.name.clone(), alias.clone()))
            .collect();
        self
    }
}

pub fn file_runtime_bindings(ast: &SourceFile, policy: ProviderRuntimePolicy) -> RuntimeBindings {
    RuntimeBindings::new(policy).with_source_types(ast)
}

pub fn type_ref_descriptor(ty: &TypeRef, runtime_bindings: &RuntimeBindings) -> TypeRefIr {
    type_text_descriptor(ty.name.trim(), runtime_bindings)
}

pub fn type_text_descriptor(ty: &str, runtime_bindings: &RuntimeBindings) -> TypeRefIr {
    if let Some(inner) = ty.strip_suffix('?') {
        return TypeRefIr::Nullable {
            inner: Box::new(type_text_descriptor(inner.trim(), runtime_bindings)),
        };
    }

    let union = split_top_level(ty, '|');
    if union.len() > 1 {
        return TypeRefIr::Union {
            items: union
                .iter()
                .map(|part| type_text_descriptor(part, runtime_bindings))
                .collect(),
        };
    }

    if let Some(value) = string_literal(ty) {
        return TypeRefIr::Literal {
            value: LiteralIr::String { value },
        };
    }

    if ty.starts_with('{') && ty.ends_with('}') {
        return TypeRefIr::Record {
            fields: record_type_descriptor_fields(ty, runtime_bindings),
        };
    }

    if let Some(parts) = generic_parts(ty) {
        let args = parts
            .args
            .iter()
            .map(|arg| type_text_descriptor(arg, runtime_bindings))
            .collect();
        return named_type_descriptor(parts.root, args, runtime_bindings);
    }

    named_type_descriptor(ty, Vec::new(), runtime_bindings)
}

fn named_type_descriptor(
    name: &str,
    args: Vec<TypeRefIr>,
    runtime_bindings: &RuntimeBindings,
) -> TypeRefIr {
    let canonical = canonical_package_type_name(name, runtime_bindings);
    let registry = prelude_registry();
    if is_language_primitive(&canonical) || registry.is_native_type_name(&canonical) {
        return TypeRefIr::Native {
            name: canonical,
            args,
        };
    }
    if let Some(symbol_path) = registry.known_type_symbol(&canonical) {
        if let Some(native_name) = canonical_native_prelude_symbol(&symbol_path) {
            return TypeRefIr::Native {
                name: native_name,
                args,
            };
        }
        return TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
                },
                symbol_path,
                abi_expectation: None,
            },
        };
    }
    TypeRefIr::Native {
        name: canonical,
        args,
    }
}

fn is_language_primitive(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "number"
            | "integer"
            | "bool"
            | "boolean"
            | "null"
            | "unknown"
            | "void"
            | "never"
    )
}

fn canonical_native_prelude_symbol(symbol: &str) -> Option<String> {
    match symbol {
        "std.collection.Array" => Some("Array".to_string()),
        "std.collection.Map" => Some("Map".to_string()),
        "std.stream.Stream" => Some("Stream".to_string()),
        "std.bytes.bytes" => Some("bytes".to_string()),
        "std.date.Date" | "Date" => Some("Date".to_string()),
        "Json" => Some("Json".to_string()),
        "JsonObject" => Some("JsonObject".to_string()),
        "config.Config" | "Config" => Some("Config".to_string()),
        other if prelude_registry().is_native_type_name(other) => Some(other.to_string()),
        _ => None,
    }
}

fn canonical_package_type_name(name: &str, runtime_bindings: &RuntimeBindings) -> String {
    canonical_package_path(name.trim(), runtime_bindings)
}

fn canonical_package_path(path: &str, runtime_bindings: &RuntimeBindings) -> String {
    if let Some(std_private_path) = canonical_std_private_root_path(path, runtime_bindings) {
        return std_private_path;
    }

    if !path.contains('.')
        && runtime_bindings
            .module_path
            .as_deref()
            .is_some_and(|module_path| module_path.starts_with("std."))
        && (runtime_bindings.type_decls.contains_key(path)
            || runtime_bindings.aliases.contains_key(path))
    {
        return format!(
            "{}.{path}",
            runtime_bindings
                .module_path
                .as_deref()
                .expect("checked module path")
        );
    }

    if !path.contains('.') {
        if runtime_bindings.policy.package_id.is_some()
            && runtime_bindings.module_path.is_some()
            && (runtime_bindings.type_decls.contains_key(path)
                || runtime_bindings.aliases.contains_key(path))
        {
            return format!(
                "{}.{path}",
                runtime_bindings
                    .module_path
                    .as_deref()
                    .expect("checked module path")
            );
        }
        return path.to_string();
    }
    PackageExportResolver::new(&runtime_bindings.package_aliases)
        .canonical_alias_path(path)
        .unwrap_or_else(|| path.to_string())
}

fn canonical_std_private_root_path(
    path: &str,
    runtime_bindings: &RuntimeBindings,
) -> Option<String> {
    let package_id = runtime_bindings.policy.package_id.as_deref()?;
    if !is_standard_package_id(package_id) {
        return None;
    }
    let rest = path.strip_prefix("root.")?;
    if rest.is_empty() {
        return None;
    }
    Some(format!("std.{rest}"))
}

fn record_type_descriptor_fields(
    ty: &str,
    runtime_bindings: &RuntimeBindings,
) -> BTreeMap<String, TypeRefIr> {
    let mut fields = BTreeMap::new();
    let Some(inner) = ty
        .trim()
        .strip_prefix('{')
        .and_then(|ty| ty.strip_suffix('}'))
    else {
        return fields;
    };
    if inner.trim().is_empty() {
        return fields;
    }
    for part in split_top_level(inner, ',') {
        let (name, field_type) = part
            .split_once(':')
            .expect("record descriptor field must contain ':' after parser validation");
        fields.insert(
            name.trim().to_string(),
            type_text_descriptor(field_type.trim(), runtime_bindings),
        );
    }
    fields
}

pub fn lower_prelude_type_decl(ty: &TypeDecl) -> TypeDeclIr {
    let type_params = ty.type_params.iter().cloned().collect::<BTreeSet<_>>();
    let bindings = RuntimeBindings::new(ProviderRuntimePolicy::disabled());
    let descriptor = if let Some(alias) = &ty.alias {
        let target = prelude_type_text_descriptor(alias.name.trim(), &bindings, &type_params);
        match target {
            TypeRefIr::Union { items } => TypeDescriptorIr::Union { variants: items },
            other => TypeDescriptorIr::Alias { target: other },
        }
    } else if ty.discriminator.is_some() {
        TypeDescriptorIr::Union {
            variants: ty
                .fields
                .iter()
                .map(|f| prelude_type_text_descriptor(f.ty.name.trim(), &bindings, &type_params))
                .collect(),
        }
    } else {
        TypeDescriptorIr::Record {
            fields: ty
                .fields
                .iter()
                .map(|f| {
                    (
                        f.name.clone(),
                        prelude_type_text_descriptor(f.ty.name.trim(), &bindings, &type_params),
                    )
                })
                .collect(),
        }
    };
    TypeDeclIr {
        name: ty.name.clone(),
        descriptor,
        type_params: ty.type_params.clone(),
        discriminator: ty.discriminator.clone(),
        implements: Vec::new(),
        source_span: None,
    }
}

fn prelude_type_text_descriptor(
    ty: &str,
    runtime_bindings: &RuntimeBindings,
    type_params: &BTreeSet<String>,
) -> TypeRefIr {
    if type_params.contains(ty.trim()) {
        return TypeRefIr::TypeParam {
            name: ty.trim().to_string(),
        };
    }
    let mut descriptor = type_text_descriptor(ty, runtime_bindings);
    substitute_prelude_type_params_in_ir(&mut descriptor, type_params);
    descriptor
}

fn substitute_prelude_type_params_in_ir(ty: &mut TypeRefIr, type_params: &BTreeSet<String>) {
    match ty {
        TypeRefIr::Native { name, args } => {
            if args.is_empty() && type_params.contains(name) {
                *ty = TypeRefIr::TypeParam { name: name.clone() };
                return;
            }
            for arg in args {
                substitute_prelude_type_params_in_ir(arg, type_params);
            }
        }
        TypeRefIr::Record { fields } => {
            for field_ty in fields.values_mut() {
                substitute_prelude_type_params_in_ir(field_ty, type_params);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                substitute_prelude_type_params_in_ir(item, type_params);
            }
        }
        TypeRefIr::Nullable { inner } => {
            substitute_prelude_type_params_in_ir(inner, type_params);
        }
        TypeRefIr::AnyInterface { interface } => {
            for arg in &mut interface.canonical_type_args {
                substitute_prelude_type_params_in_ir(arg, type_params);
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                substitute_prelude_type_params_in_ir(&mut param.ty, type_params);
            }
            substitute_prelude_type_params_in_ir(return_type, type_params);
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
}
