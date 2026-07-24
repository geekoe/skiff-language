use std::collections::BTreeMap;

use skiff_artifact_model::{NativeSignatureDef, STD_NATIVE_SIGNATURES};

#[derive(Clone, Copy, Debug)]
pub struct NativeTypeArgRef<'a> {
    pub key: &'a str,
    pub unresolved_type_param: Option<&'a str>,
}

impl<'a> NativeTypeArgRef<'a> {
    pub fn new(key: &'a str, unresolved_type_param: Option<&'a str>) -> Self {
        Self {
            key,
            unresolved_type_param,
        }
    }
}

pub fn validate_native_call_arg_count(
    signature: &NativeSignatureDef,
    arg_count: usize,
) -> std::result::Result<(), String> {
    if arg_count == signature.params.len() {
        return Ok(());
    }
    Err(format!(
        "expected {} args, got {arg_count}",
        signature.params.len()
    ))
}

pub fn validate_native_call_type_arg_refs<'a>(
    signature: &NativeSignatureDef,
    type_args: impl IntoIterator<Item = NativeTypeArgRef<'a>>,
) -> Option<String> {
    let type_args = type_args
        .into_iter()
        .map(|arg| (arg.key, arg.unresolved_type_param))
        .collect::<BTreeMap<_, _>>();
    validate_native_call_type_arg_keys(signature, type_args.keys().copied())
        .err()
        .or_else(|| validate_native_call_type_arg_unresolved(signature, &type_args))
}

pub fn type_arg_key(index: usize) -> String {
    format!("T{index}")
}

pub fn is_reserved_std_native_target(target: &str) -> bool {
    target == "std"
        || target.starts_with("std.")
        || STD_NATIVE_SIGNATURES
            .iter()
            .any(|signature| signature.binding_key == target)
}

fn validate_native_call_type_arg_keys<'a>(
    signature: &NativeSignatureDef,
    type_arg_keys: impl IntoIterator<Item = &'a str>,
) -> std::result::Result<(), String> {
    let type_arg_keys = type_arg_keys.into_iter().collect::<Vec<_>>();
    if signature.type_param_count == 0 {
        if let Some(extra) = type_arg_keys.first() {
            return Err(format!("unexpected generic typeArgs.{extra}"));
        }
        return Ok(());
    }

    for key in &type_arg_keys {
        match native_type_arg_index(key) {
            Some(index) if index < signature.type_param_count => {}
            Some(index) => return Err(format!("unexpected generic typeArgs[{index}]")),
            None => return Err(format!("unexpected generic typeArgs.{key}")),
        }
    }
    for key in &type_arg_keys {
        if let Some(index) = native_type_arg_index(key) {
            if index > 0
                && !type_arg_keys
                    .iter()
                    .any(|key| *key == type_arg_key(index - 1))
            {
                return Err(format!(
                    "typeArgs[{index}] is present without typeArgs[{}]",
                    index - 1
                ));
            }
        }
    }
    for index in 0..signature.type_param_count {
        let key = type_arg_key(index);
        if !type_arg_keys.iter().any(|item| *item == key) {
            return Err(format!("missing generic typeArgs[{index}]"));
        }
    }
    Ok(())
}

fn validate_native_call_type_arg_unresolved(
    signature: &NativeSignatureDef,
    type_args: &BTreeMap<&str, Option<&str>>,
) -> Option<String> {
    for index in 0..signature.type_param_count {
        let key = type_arg_key(index);
        if let Some(Some(name)) = type_args.get(key.as_str()) {
            return Some(format!("unresolved typeArgs[{index}] {name}"));
        }
    }
    None
}

fn native_type_arg_index(key: &str) -> Option<usize> {
    key.strip_prefix('T')?.parse().ok()
}
