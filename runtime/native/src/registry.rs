use serde_json::Value;

use crate::error::{Result, RuntimeError};

use self::table::{handler_entries, validate_builtin_handlers as validate_handler_table};

#[derive(Clone, Debug, Default)]
pub struct NativeRegistry;

impl NativeRegistry {
    pub fn is_registered(&self, target_or_callee: &str) -> bool {
        debug_assert!(
            Self::validate_builtin_handlers().is_ok(),
            "native handler registry table should validate"
        );

        handler_entries()
            .iter()
            .any(|binding| binding.matches(target_or_callee))
    }

    pub fn dispatch(&self, target_or_callee: &str, args: &[Value]) -> Result<Option<Value>> {
        debug_assert!(
            Self::validate_builtin_handlers().is_ok(),
            "native handler registry table should validate"
        );

        let Some(binding) = handler_entries()
            .iter()
            .find(|binding| binding.matches(target_or_callee))
        else {
            return Ok(None);
        };

        binding.dispatch(args).map(Some)
    }

    pub fn unsupported(&self, target_or_callee: &str) -> RuntimeError {
        RuntimeError::Unsupported(format!("unsupported native target {target_or_callee}"))
    }

    pub(crate) fn validate_builtin_handlers() -> table::RegistryValidationResult {
        validate_handler_table()
    }
}

mod table;
#[cfg(test)]
mod tests;
