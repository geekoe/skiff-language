use crate::call_helpers::runtime_string_arg;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{runtime_map_has, RequestHeap, RuntimeValue};

pub(super) struct BuiltinDispatch;

impl BuiltinDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "object.has"
                | "string.length"
                | "string.contains"
                | "string.replaceAll"
                | "string.concat"
                | "string.startsWith"
                | "string.endsWith"
                | "string.lowercase"
        )
    }

    pub(super) fn dispatch(
        target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        match target {
            "object.has" => {
                let object = args.first().ok_or_else(|| {
                    RuntimeError::Decode("object.has requires object argument".to_string())
                })?;
                let key = args.get(1).ok_or_else(|| {
                    RuntimeError::Decode("object.has requires key argument".to_string())
                })?;
                let key = runtime_string_arg(key, "object.has key")?;
                let key = RuntimeValue::String(key.to_string());
                Ok(RuntimeValue::Bool(runtime_map_has(heap, object, &key)?))
            }
            "string.length" => {
                let value = args.first().ok_or_else(|| {
                    RuntimeError::Decode("string.length requires string argument".to_string())
                })?;
                Ok(RuntimeValue::Number(
                    runtime_string_arg(value, "string.length value")?
                        .chars()
                        .count() as f64,
                ))
            }
            "string.contains" => {
                let value = runtime_string_arg(
                    args.first().ok_or_else(|| {
                        RuntimeError::Decode("string.contains requires string argument".to_string())
                    })?,
                    "string.contains value",
                )?;
                let needle = runtime_string_arg(
                    args.get(1).ok_or_else(|| {
                        RuntimeError::Decode("string.contains requires needle".to_string())
                    })?,
                    "string.contains needle",
                )?;
                Ok(RuntimeValue::Bool(value.contains(needle)))
            }
            "string.replaceAll" => {
                let value = runtime_string_arg(
                    args.first().ok_or_else(|| {
                        RuntimeError::Decode(
                            "string.replaceAll requires string argument".to_string(),
                        )
                    })?,
                    "string.replaceAll value",
                )?;
                let needle = runtime_string_arg(
                    args.get(1).ok_or_else(|| {
                        RuntimeError::Decode("string.replaceAll requires needle".to_string())
                    })?,
                    "string.replaceAll needle",
                )?;
                let replacement = runtime_string_arg(
                    args.get(2).ok_or_else(|| {
                        RuntimeError::Decode("string.replaceAll requires replacement".to_string())
                    })?,
                    "string.replaceAll replacement",
                )?;
                Ok(RuntimeValue::String(value.replace(needle, replacement)))
            }
            "string.concat" => {
                let left = runtime_string_arg(
                    args.first().ok_or_else(|| {
                        RuntimeError::Decode("string.concat requires left argument".to_string())
                    })?,
                    "string.concat left",
                )?;
                let right = runtime_string_arg(
                    args.get(1).ok_or_else(|| {
                        RuntimeError::Decode("string.concat requires right argument".to_string())
                    })?,
                    "string.concat right",
                )?;
                Ok(RuntimeValue::String(format!("{left}{right}")))
            }
            "string.startsWith" => {
                let value = runtime_string_arg(
                    args.first().ok_or_else(|| {
                        RuntimeError::Decode(
                            "string.startsWith requires string argument".to_string(),
                        )
                    })?,
                    "string.startsWith value",
                )?;
                let prefix = runtime_string_arg(
                    args.get(1).ok_or_else(|| {
                        RuntimeError::Decode("string.startsWith requires prefix".to_string())
                    })?,
                    "string.startsWith prefix",
                )?;
                Ok(RuntimeValue::Bool(value.starts_with(prefix)))
            }
            "string.endsWith" => {
                let value = runtime_string_arg(
                    args.first().ok_or_else(|| {
                        RuntimeError::Decode("string.endsWith requires string argument".to_string())
                    })?,
                    "string.endsWith value",
                )?;
                let suffix = runtime_string_arg(
                    args.get(1).ok_or_else(|| {
                        RuntimeError::Decode("string.endsWith requires suffix".to_string())
                    })?,
                    "string.endsWith suffix",
                )?;
                Ok(RuntimeValue::Bool(value.ends_with(suffix)))
            }
            "string.lowercase" => {
                let value = runtime_string_arg(
                    args.first().ok_or_else(|| {
                        RuntimeError::Decode(
                            "string.lowercase requires string argument".to_string(),
                        )
                    })?,
                    "string.lowercase value",
                )?;
                Ok(RuntimeValue::String(value.to_lowercase()))
            }
            _ => unreachable!("builtin target checked by caller"),
        }
    }
}
