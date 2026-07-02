use std::collections::BTreeSet;

use crate::{
    shared::ast::{FunctionDecl, InterfaceOperation, TypeRef},
    shared::parser::parse_source,
    shared::type_syntax::{generic_args, generic_parts, split_top_level, string_literal},
};

use super::{
    compiler_owned_schema_stable_type, qualified_prelude_type, schema_primitive_type, type_root,
    PreludeRegistry,
};

pub(super) use skiff_compiler_core::prelude_registry::validate_root_projection_metadata;

impl PreludeRegistry {
    pub(super) fn validate_export_modules(&self) -> Result<(), String> {
        let source_modules = self.source_modules.iter().collect::<BTreeSet<_>>();
        for module in &self.export_modules {
            if !source_modules.contains(module) {
                return Err(format!(
                    "exports includes {module}, but no standard_library source module provides it"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_schema_stable_types(&self) -> Result<(), String> {
        for name in &self.schema_stable_types {
            if compiler_owned_schema_stable_type(name)
                || self.type_decl(name).is_some()
                || self.type_alias(name).is_some()
            {
                continue;
            }
            return Err(format!(
                "schema.schemaStableTypes includes {name}, but no standard_library source type declaration provides it"
            ));
        }
        for name in &self.schema_stable_types {
            let Some(decl) = self.type_decl(name) else {
                continue;
            };
            if let Some(alias) = &decl.alias {
                self.validate_schema_stable_type_ref(name, alias, &decl.type_params)?;
            }
            for field in &decl.fields {
                self.validate_schema_stable_type_ref(name, &field.ty, &decl.type_params)?;
            }
        }
        for (name, alias) in &self.type_aliases {
            if self.is_schema_stable_type(name) {
                self.validate_schema_stable_type_ref(name, &alias.target_type, &[])?;
            }
        }
        Ok(())
    }

    fn validate_schema_stable_type_ref(
        &self,
        owner: &str,
        ty: &TypeRef,
        type_params: &[String],
    ) -> Result<(), String> {
        self.validate_schema_stable_type_name(owner, &ty.name, type_params)
    }

    fn validate_schema_stable_type_name(
        &self,
        owner: &str,
        raw_name: &str,
        type_params: &[String],
    ) -> Result<(), String> {
        let name = raw_name.trim();
        if name.is_empty() || name == "{}" || is_string_literal_type(name) {
            return Ok(());
        }
        if name.starts_with("fn(") {
            return Err(format!(
                "schema-stable standard_library type {owner} cannot reference callback function type {name}"
            ));
        }
        let nullable_stripped = name.trim_end_matches('?').trim();
        if nullable_stripped != name {
            return self.validate_schema_stable_type_name(owner, nullable_stripped, type_params);
        }
        let union = split_top_level(name, '|');
        if union.len() > 1 {
            for part in union {
                self.validate_schema_stable_type_name(owner, part, type_params)?;
            }
            return Ok(());
        }
        if name.starts_with('{') && name.ends_with('}') {
            let inner = &name[1..name.len() - 1];
            for field in split_top_level(inner, ',') {
                let Some((_, field_type)) = field.split_once(':') else {
                    continue;
                };
                self.validate_schema_stable_type_name(owner, field_type, type_params)?;
            }
            return Ok(());
        }
        if let Some(parts) = generic_parts(name) {
            self.validate_schema_stable_type_root(owner, parts.root, type_params)?;
            for arg in parts.args {
                self.validate_schema_stable_type_name(owner, arg, type_params)?;
            }
            return Ok(());
        }
        self.validate_schema_stable_type_root(owner, name, type_params)
    }

    fn validate_schema_stable_type_root(
        &self,
        owner: &str,
        root: &str,
        type_params: &[String],
    ) -> Result<(), String> {
        let root = type_root(root);
        if schema_primitive_type(root)
            || type_params.iter().any(|param| param == root)
            || self.is_schema_stable_type(root)
            || compiler_owned_schema_stable_type(root)
        {
            return Ok(());
        }
        if self.type_decls.contains_key(root) || self.known_type_symbol(root).is_some() {
            return Err(format!(
                "schema-stable standard_library type {owner} references non schema-stable standard_library type {root}"
            ));
        }
        Err(format!(
            "schema-stable standard_library type {owner} references unknown type {root}"
        ))
    }

    pub(super) fn validate_source_type_refs(
        &self,
        module_path: &str,
        text: &str,
    ) -> Result<(), String> {
        let source = parse_source(text).map_err(|error| error.to_string())?;
        let known = self.known_type_roots();

        for ty in &source.types {
            let type_params = ty.type_params.clone();
            for implemented in &ty.implements {
                validate_type_ref(implemented, &known, &type_params, module_path)?;
            }
            if let Some(alias) = &ty.alias {
                validate_type_ref(alias, &known, &type_params, module_path)?;
            }
            for field in &ty.fields {
                validate_type_ref(&field.ty, &known, &type_params, module_path)?;
            }
        }
        for alias in &source.aliases {
            validate_type_ref(&alias.target_type, &known, &[], module_path)?;
        }
        for operation in &source.function_signatures {
            validate_operation_type_refs(operation, &known, &[], module_path)?;
        }
        for function in &source.functions {
            validate_function_type_refs(function, &known, &[], module_path)?;
        }
        for implementation in &source.impls {
            let impl_type_params = generic_type_params(&implementation.target);
            validate_type_name(
                &implementation.target,
                &known,
                &impl_type_params,
                module_path,
            )?;
            for method in &implementation.methods {
                validate_operation_type_refs(method, &known, &impl_type_params, module_path)?;
            }
            for method in &implementation.method_bodies {
                validate_function_type_refs(method, &known, &impl_type_params, module_path)?;
            }
        }
        Ok(())
    }

    fn known_type_roots(&self) -> Vec<String> {
        let mut known = self.prelude_types.clone();
        known.extend(self.type_decls.keys().cloned());
        known.extend(self.type_aliases.keys().cloned());
        known.extend(super::primitive_type_symbols().into_keys());
        known.sort();
        known.dedup();
        known
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::validate_root_projection_metadata;

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
}

fn validate_operation_type_refs(
    operation: &InterfaceOperation,
    known: &[String],
    inherited_type_params: &[String],
    module_path: &str,
) -> Result<(), String> {
    let mut type_params = inherited_type_params.to_vec();
    type_params.extend(operation.type_params.iter().cloned());
    for param in &operation.params {
        validate_type_ref(&param.ty, known, &type_params, module_path)?;
    }
    validate_type_ref(&operation.return_type, known, &type_params, module_path)
}

fn validate_function_type_refs(
    function: &FunctionDecl,
    known: &[String],
    inherited_type_params: &[String],
    module_path: &str,
) -> Result<(), String> {
    let mut type_params = inherited_type_params.to_vec();
    type_params.extend(function.type_params.iter().cloned());
    for param in &function.params {
        validate_type_ref(&param.ty, known, &type_params, module_path)?;
    }
    validate_type_ref(&function.return_type, known, &type_params, module_path)
}

fn validate_type_ref(
    ty: &TypeRef,
    known: &[String],
    type_params: &[String],
    module_path: &str,
) -> Result<(), String> {
    validate_type_name(&ty.name, known, type_params, module_path)
}

fn validate_type_name(
    raw_name: &str,
    known: &[String],
    type_params: &[String],
    module_path: &str,
) -> Result<(), String> {
    let name = raw_name.trim();
    if name.is_empty() || name == "{}" || is_string_literal_type(name) {
        return Ok(());
    }
    if let Some(inner) = name.strip_prefix("fn(") {
        return validate_function_type_tail(inner, known, type_params, module_path);
    }

    let nullable_stripped = name.trim_end_matches('?').trim();
    if nullable_stripped != name {
        return validate_type_name(nullable_stripped, known, type_params, module_path);
    }

    for part in split_top_level(name, '|') {
        let part = part.trim();
        if part != name {
            validate_type_name(part, known, type_params, module_path)?;
        }
    }
    if split_top_level(name, '|').len() > 1 {
        return Ok(());
    }

    if name.starts_with('{') && name.ends_with('}') {
        let inner = &name[1..name.len() - 1];
        for field in split_top_level(inner, ',') {
            let Some((_, field_type)) = field.split_once(':') else {
                continue;
            };
            validate_type_name(field_type, known, type_params, module_path)?;
        }
        return Ok(());
    }

    if let Some(parts) = generic_parts(name) {
        validate_named_type_root(parts.root, known, type_params, module_path)?;
        for arg in parts.args {
            validate_type_name(arg, known, type_params, module_path)?;
        }
        return Ok(());
    }

    validate_named_type_root(name, known, type_params, module_path)
}

fn validate_function_type_tail(
    tail: &str,
    known: &[String],
    type_params: &[String],
    module_path: &str,
) -> Result<(), String> {
    let Some((params, return_type)) = tail.split_once(") ->") else {
        return Err(format!("invalid function type fn({tail}"));
    };
    for param in split_top_level(params, ',') {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }
        let Some((_, param_type)) = param.split_once(':') else {
            return Err(format!("invalid function type parameter `{param}`"));
        };
        validate_type_name(param_type, known, type_params, module_path)?;
    }
    validate_type_name(return_type, known, type_params, module_path)
}

fn validate_named_type_root(
    root: &str,
    known: &[String],
    type_params: &[String],
    module_path: &str,
) -> Result<(), String> {
    let root = type_root(root);
    if root.is_empty()
        || known.iter().any(|known| known == root)
        || qualified_prelude_type(root)
            .is_some_and(|(_, bare)| known.iter().any(|known| known == bare))
        || type_params.iter().any(|param| param == root)
    {
        return Ok(());
    }
    Err(format!(
        "unknown standard_library type reference {root} in module {module_path}"
    ))
}

fn generic_type_params(name: &str) -> Vec<String> {
    generic_args(name)
        .map(|args| {
            args.into_iter()
                .map(str::trim)
                .filter(|arg| {
                    !arg.is_empty()
                        && arg
                            .chars()
                            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn is_string_literal_type(name: &str) -> bool {
    string_literal(name).is_some()
}
