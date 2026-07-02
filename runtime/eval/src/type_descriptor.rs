//! Interpreter type substitution helpers.

use std::collections::BTreeMap;

#[cfg(any(test, feature = "test-support"))]
use serde_json::{json, Value};
use skiff_runtime_boundary::type_descriptor as runtime_type_descriptor;
use skiff_runtime_linked_program::LinkedTypeRef;
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_linked_program::type_ref_to_value;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TypeSubstitutions {
    entries: BTreeMap<String, LinkedTypeRef>,
}

impl TypeSubstitutions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&LinkedTypeRef> {
        self.entries.get(name)
    }

    pub fn insert(&mut self, name: impl Into<String>, value: LinkedTypeRef) {
        self.entries.insert(name.into(), value);
    }

    pub fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
    }

    pub fn as_linked_map(&self) -> &BTreeMap<String, LinkedTypeRef> {
        &self.entries
    }

    #[cfg(any(test, feature = "test-support"))]
    fn descriptor_value(&self, name: &str) -> Option<Value> {
        self.entries.get(name).map(type_ref_to_value)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn collect_type_substitutions(
    pattern: &Value,
    actual: &Value,
    substitutions: &mut TypeSubstitutions,
) {
    if let Some(name) = runtime_type_descriptor::type_parameter_name(pattern) {
        if let Some(actual) = resolved_substitution_actual(actual, substitutions) {
            substitutions.insert(name.to_string(), actual);
        }
        return;
    }

    if let Some(pattern_text) = pattern.as_str() {
        let pattern_descriptor = type_text_descriptor(pattern_text);
        collect_type_substitutions(&pattern_descriptor, actual, substitutions);
        return;
    }

    if let Some(actual_text) = actual.as_str() {
        let actual_descriptor = type_text_descriptor(actual_text);
        collect_type_substitutions(pattern, &actual_descriptor, substitutions);
        return;
    }

    let Some(pattern_object) = pattern.as_object() else {
        return;
    };
    let Some(actual_object) = actual.as_object() else {
        return;
    };

    if runtime_type_descriptor::descriptor_kind_has_type_args(pattern_object)
        && runtime_type_descriptor::descriptor_kind_has_type_args(actual_object)
    {
        let pattern_name = pattern_object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual_name = actual_object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !type_name_matches(pattern_name, actual_name) {
            return;
        }
        if let (Some(pattern_args), Some(actual_args)) = (
            pattern_object.get("args").and_then(Value::as_array),
            actual_object.get("args").and_then(Value::as_array),
        ) {
            for (pattern_arg, actual_arg) in pattern_args.iter().zip(actual_args) {
                collect_type_substitutions(pattern_arg, actual_arg, substitutions);
            }
        }
        return;
    }

    if matches!(
        runtime_type_descriptor::descriptor_kind(pattern),
        Some("nullable")
    ) && matches!(
        runtime_type_descriptor::descriptor_kind(actual),
        Some("nullable")
    ) {
        if let (Some(pattern_inner), Some(actual_inner)) =
            (pattern_object.get("inner"), actual_object.get("inner"))
        {
            collect_type_substitutions(pattern_inner, actual_inner, substitutions);
        }
        return;
    }

    if matches!(
        runtime_type_descriptor::descriptor_kind(pattern),
        Some("union")
    ) && matches!(
        runtime_type_descriptor::descriptor_kind(actual),
        Some("union")
    ) {
        if let (Some(pattern_types), Some(actual_types)) = (
            runtime_type_descriptor::descriptor_union_types(pattern),
            runtime_type_descriptor::descriptor_union_types(actual),
        ) {
            for (pattern_type, actual_type) in pattern_types.iter().zip(actual_types) {
                collect_type_substitutions(pattern_type, actual_type, substitutions);
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)]
pub fn collect_type_text_substitutions(
    pattern_text: &str,
    actual: &Value,
    substitutions: &mut TypeSubstitutions,
) {
    let pattern_descriptor = type_text_descriptor(pattern_text);
    collect_type_substitutions(&pattern_descriptor, actual, substitutions);
}

#[cfg(any(test, feature = "test-support"))]
pub fn substitute_type_descriptor(
    expected_type: Option<&Value>,
    substitutions: &TypeSubstitutions,
) -> Option<Value> {
    let expected_type = expected_type?;
    if substitutions.is_empty() {
        return None;
    }
    Some(substitute_type_value(expected_type, substitutions))
}

#[cfg(any(test, feature = "test-support"))]
pub fn substitute_type_value(value: &Value, substitutions: &TypeSubstitutions) -> Value {
    if let Some(name) = runtime_type_descriptor::type_parameter_name(value) {
        if let Some(replacement) = substitutions.descriptor_value(name) {
            return replacement;
        }
    }

    if let Some(text) = value.as_str() {
        return type_text_descriptor_with_substitutions(text, substitutions);
    }

    let Some(object) = value.as_object() else {
        return value.clone();
    };

    let mut result = object.clone();
    if let Some(args) = object.get("args").and_then(Value::as_array) {
        result.insert(
            "args".to_string(),
            Value::Array(
                args.iter()
                    .map(|arg| substitute_type_value(arg, substitutions))
                    .collect(),
            ),
        );
    }
    if let Some(inner) = object.get("inner") {
        result.insert(
            "inner".to_string(),
            substitute_type_value(inner, substitutions),
        );
    }
    if let Some(types) = object.get("items").and_then(Value::as_array) {
        result.insert(
            "items".to_string(),
            Value::Array(
                types
                    .iter()
                    .map(|ty| substitute_type_value(ty, substitutions))
                    .collect(),
            ),
        );
    }
    if let Some(fields) = object.get("fields").and_then(Value::as_array) {
        result.insert(
            "fields".to_string(),
            Value::Array(
                fields
                    .iter()
                    .map(|field| substitute_field_type(field, substitutions))
                    .collect(),
            ),
        );
    }
    if let Some(representation) = object.get("representation") {
        result.insert(
            "representation".to_string(),
            substitute_type_value(representation, substitutions),
        );
    }
    Value::Object(result)
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_text_descriptor(text: &str) -> Value {
    runtime_type_descriptor::type_text_descriptor(text)
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_ref_name(value: &Value) -> Option<String> {
    runtime_type_descriptor::type_ref_name(value)
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_ref_is_union(value: &Value) -> bool {
    runtime_type_descriptor::type_ref_is_union(value)
}

pub fn type_name_root(name: &str) -> &str {
    runtime_type_descriptor::type_name_root(name)
}

pub fn short_type_name(name: &str) -> &str {
    runtime_type_descriptor::short_type_name(name)
}

pub fn type_name_matches(pattern: &str, actual: &str) -> bool {
    runtime_type_descriptor::type_name_matches(pattern, actual)
}

pub fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    runtime_type_descriptor::split_top_level(input, delimiter)
}

#[cfg(all(test, any()))]
pub fn type_substitutions_from_receiver(pattern: &str, actual: &str) -> TypeSubstitutions {
    type_substitutions_from_receiver_matching(pattern, actual, type_name_matches)
}

pub fn provider_type_substitutions_from_receiver(
    pattern: &str,
    actual: &str,
    module_name: Option<&str>,
) -> TypeSubstitutions {
    type_substitutions_from_receiver_matching(pattern, actual, |pattern_root, actual_root| {
        provider_receiver_type_name_matches(pattern_root, actual_root, module_name)
    })
}

fn type_substitutions_from_receiver_matching(
    pattern: &str,
    actual: &str,
    type_name_matches: impl Fn(&str, &str) -> bool,
) -> TypeSubstitutions {
    let Some((pattern_root, pattern_args)) = runtime_type_descriptor::generic_text_parts(pattern)
    else {
        return TypeSubstitutions::new();
    };
    let Some((actual_root, actual_args)) = runtime_type_descriptor::generic_text_parts(actual)
    else {
        return TypeSubstitutions::new();
    };
    if !type_name_matches(pattern_root, actual_root) || pattern_args.len() != actual_args.len() {
        return TypeSubstitutions::new();
    }
    let mut substitutions = TypeSubstitutions::new();
    for (name, actual) in pattern_args.into_iter().zip(actual_args) {
        if runtime_type_descriptor::is_type_parameter(name) {
            substitutions.insert(name.to_string(), linked_type_ref_from_text(actual));
        }
    }
    substitutions
}

fn provider_receiver_type_name_matches(
    pattern: &str,
    actual: &str,
    module_name: Option<&str>,
) -> bool {
    if type_name_matches(pattern, actual) {
        return true;
    }
    let pattern = pattern.trim();
    let actual = actual.trim();
    let Some(module_name) = module_name.map(str::trim).filter(|name| !name.is_empty()) else {
        return false;
    };
    if !pattern.contains('.') && format!("{module_name}.{pattern}") == actual {
        return true;
    }
    pattern
        .strip_prefix(&format!("{module_name}."))
        .is_some_and(|pattern_bare| pattern_bare == actual && !actual.contains('.'))
}

#[cfg(any(test, feature = "test-support"))]
fn substitute_field_type(field: &Value, substitutions: &TypeSubstitutions) -> Value {
    let Some(object) = field.as_object() else {
        return field.clone();
    };
    let mut result = object.clone();
    if let Some(ty) = object.get("type") {
        result.insert("type".to_string(), substitute_type_value(ty, substitutions));
    }
    Value::Object(result)
}

#[cfg(any(test, feature = "test-support"))]
fn type_text_descriptor_with_substitutions(text: &str, substitutions: &TypeSubstitutions) -> Value {
    let text = text.trim();
    let union_parts = split_top_level(text, '|');
    if union_parts.len() > 1 {
        return json!({
            "kind": "union",
            "items": union_parts
                .into_iter()
                .map(|part| type_text_descriptor_with_substitutions(part, substitutions))
                .collect::<Vec<_>>(),
        });
    }
    if let Some(inner) = runtime_type_descriptor::strip_top_level_nullable_suffix(text) {
        return json!({
            "kind": "nullable",
            "inner": type_text_descriptor_with_substitutions(inner, substitutions),
        });
    }
    if let Some((root, args)) = runtime_type_descriptor::generic_text_parts(text) {
        return json!({
            "kind": "builtin",
            "name": root,
            "args": args.into_iter().map(|arg| {
                substitutions
                    .descriptor_value(arg.trim())
                    .unwrap_or_else(|| type_text_descriptor_with_substitutions(arg, substitutions))
            }).collect::<Vec<_>>(),
        });
    }
    substitutions
        .descriptor_value(text)
        .unwrap_or_else(|| type_text_descriptor(text))
}

#[cfg(any(test, feature = "test-support"))]
fn resolved_substitution_actual(
    actual: &Value,
    substitutions: &TypeSubstitutions,
) -> Option<LinkedTypeRef> {
    if let Some(actual_name) = runtime_type_descriptor::type_parameter_name(actual) {
        return substitutions.get(actual_name).cloned();
    }
    linked_type_ref_from_descriptor(actual)
}

#[cfg(any(test, feature = "test-support"))]
fn linked_type_ref_from_descriptor(value: &Value) -> Option<LinkedTypeRef> {
    if let Some(text) = value.as_str() {
        return Some(linked_type_ref_from_text(text));
    }

    if let Ok(type_ref) = serde_json::from_value::<LinkedTypeRef>(value.clone()) {
        return Some(type_ref);
    }

    let object = value.as_object()?;
    match object.get("kind").and_then(Value::as_str) {
        Some("record") => {
            let fields = object.get("fields")?;
            let fields = match fields {
                Value::Object(fields) => fields
                    .iter()
                    .filter_map(|(name, value)| {
                        linked_type_ref_from_descriptor(value)
                            .map(|type_ref| (name.clone(), type_ref))
                    })
                    .collect::<BTreeMap<_, _>>(),
                Value::Array(fields) => fields
                    .iter()
                    .filter_map(|field| {
                        let field = field.as_object()?;
                        let name = field.get("name").and_then(Value::as_str)?;
                        let ty = field
                            .get("type")
                            .or_else(|| field.get("ty"))
                            .and_then(linked_type_ref_from_descriptor)?;
                        Some((name.to_string(), ty))
                    })
                    .collect::<BTreeMap<_, _>>(),
                _ => return None,
            };
            Some(LinkedTypeRef::Record { fields })
        }
        Some("union") => {
            let items = object
                .get("items")
                .or_else(|| object.get("variants"))?
                .as_array()?
                .iter()
                .filter_map(linked_type_ref_from_descriptor)
                .collect::<Vec<_>>();
            Some(LinkedTypeRef::Union { items })
        }
        Some("alias") => object
            .get("target")
            .and_then(linked_type_ref_from_descriptor),
        _ => None,
    }
}

fn linked_type_ref_from_text(text: &str) -> LinkedTypeRef {
    let text = text.trim();
    let union_parts = split_top_level(text, '|');
    if union_parts.len() > 1 {
        return LinkedTypeRef::Union {
            items: union_parts
                .into_iter()
                .map(linked_type_ref_from_text)
                .collect(),
        };
    }
    if let Some(inner) = runtime_type_descriptor::strip_top_level_nullable_suffix(text) {
        return LinkedTypeRef::Nullable {
            inner: Box::new(linked_type_ref_from_text(inner)),
        };
    }
    if let Some((root, args)) = runtime_type_descriptor::generic_text_parts(text) {
        return LinkedTypeRef::Native {
            name: root.to_string(),
            args: args.into_iter().map(linked_type_ref_from_text).collect(),
        };
    }
    if runtime_type_descriptor::is_type_parameter(text) {
        return LinkedTypeRef::TypeParam {
            name: text.to_string(),
        };
    }
    LinkedTypeRef::Native {
        name: text.to_string(),
        args: Vec::new(),
    }
}
