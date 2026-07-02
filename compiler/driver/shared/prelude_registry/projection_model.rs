use std::collections::{BTreeMap, BTreeSet};

use skiff_compiler_core::artifact::{
    LiteralIr, PackageRefIr, PackageSymbolRef, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
};

use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_source::prelude_registry::PreludeRegistry;
use skiff_syntax::ast::TypeDecl;
use skiff_syntax::type_syntax::{generic_parts, split_top_level, string_literal};

pub(super) fn lower_prelude_type_decl(registry: &PreludeRegistry, ty: &TypeDecl) -> TypeDeclIr {
    let type_params = ty.type_params.iter().cloned().collect::<BTreeSet<_>>();
    let descriptor = if let Some(alias) = &ty.alias {
        let target = prelude_type_text_descriptor(registry, alias.name.trim(), &type_params);
        match target {
            TypeRefIr::Union { items } => TypeDescriptorIr::Union { variants: items },
            other => TypeDescriptorIr::Alias { target: other },
        }
    } else if ty.discriminator.is_some() {
        TypeDescriptorIr::Union {
            variants: ty
                .fields
                .iter()
                .map(|field| {
                    prelude_type_text_descriptor(registry, field.ty.name.trim(), &type_params)
                })
                .collect(),
        }
    } else {
        TypeDescriptorIr::Record {
            fields: ty
                .fields
                .iter()
                .map(|field| {
                    (
                        field.name.clone(),
                        prelude_type_text_descriptor(registry, field.ty.name.trim(), &type_params),
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
    registry: &PreludeRegistry,
    ty: &str,
    type_params: &BTreeSet<String>,
) -> TypeRefIr {
    if type_params.contains(ty.trim()) {
        return TypeRefIr::TypeParam {
            name: ty.trim().to_string(),
        };
    }
    let mut descriptor = type_text_descriptor(registry, ty);
    substitute_prelude_type_params_in_ir(&mut descriptor, type_params);
    descriptor
}

fn type_text_descriptor(registry: &PreludeRegistry, ty: &str) -> TypeRefIr {
    if let Some(inner) = ty.strip_suffix('?') {
        return TypeRefIr::Nullable {
            inner: Box::new(type_text_descriptor(registry, inner.trim())),
        };
    }

    let union = split_top_level(ty, '|');
    if union.len() > 1 {
        return TypeRefIr::Union {
            items: union
                .iter()
                .map(|part| type_text_descriptor(registry, part))
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
            fields: record_type_descriptor_fields(registry, ty),
        };
    }

    if let Some(parts) = generic_parts(ty) {
        let args = parts
            .args
            .iter()
            .map(|arg| type_text_descriptor(registry, arg))
            .collect();
        return named_type_descriptor(registry, parts.root, args);
    }

    named_type_descriptor(registry, ty, Vec::new())
}

fn named_type_descriptor(
    registry: &PreludeRegistry,
    name: &str,
    args: Vec<TypeRefIr>,
) -> TypeRefIr {
    let canonical = name.trim().to_string();
    if is_language_primitive(&canonical) || registry.is_native_type_name(&canonical) {
        return TypeRefIr::Native {
            name: canonical,
            args,
        };
    }
    if let Some(symbol_path) = registry.known_type_symbol(&canonical) {
        if let Some(native_name) = canonical_native_prelude_symbol(registry, &symbol_path) {
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

fn canonical_native_prelude_symbol(registry: &PreludeRegistry, symbol: &str) -> Option<String> {
    match symbol {
        "std.collection.Array" => Some("Array".to_string()),
        "std.collection.Map" => Some("Map".to_string()),
        "std.stream.Stream" => Some("Stream".to_string()),
        "std.bytes.bytes" => Some("bytes".to_string()),
        "std.date.Date" | "Date" => Some("Date".to_string()),
        "Json" => Some("Json".to_string()),
        "JsonObject" => Some("JsonObject".to_string()),
        "config.Config" | "Config" => Some("Config".to_string()),
        other if registry.is_native_type_name(other) => Some(other.to_string()),
        _ => None,
    }
}

fn record_type_descriptor_fields(
    registry: &PreludeRegistry,
    ty: &str,
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
            type_text_descriptor(registry, field_type.trim()),
        );
    }
    fields
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
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
}
