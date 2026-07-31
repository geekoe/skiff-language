use skiff_artifact_model::{NativeSignatureDef, NativeTarget, STD_NATIVE_SIGNATURES};

use super::{
    is_reserved_std_native_target, native_target_binding_key, native_target_name,
    validate_native_call_arg_count, validate_native_call_type_arg_refs, NativeBindingSpec,
    NativeTypeArgRef,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeCallValidation {
    Known,
    External,
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeDispatchTarget<'a> {
    Resolved {
        target_name: String,
        binding_key: &'a str,
    },
    MissingExternalBinding {
        target_name: String,
    },
    Invalid(String),
}

#[derive(Clone, Copy, Debug)]
pub struct NativeSignatureRegistry {
    signatures: &'static [NativeSignatureDef],
}

impl NativeSignatureRegistry {
    pub fn builtins() -> Self {
        Self {
            signatures: STD_NATIVE_SIGNATURES,
        }
    }

    pub fn signature(&self, binding_key: &str) -> Option<&'static NativeSignatureDef> {
        self.binding_spec(binding_key).map(|spec| spec.signature)
    }

    pub fn binding_spec(&self, binding_key: &str) -> Option<NativeBindingSpec> {
        NativeBindingSpec::from_signature(
            self.signatures
                .iter()
                .find(|signature| signature.binding_key == binding_key)?,
        )
    }

    pub fn validate_native_call_artifact<'a>(
        &self,
        target: &NativeTarget,
        arg_count: usize,
        type_args: impl IntoIterator<Item = NativeTypeArgRef<'a>>,
    ) -> NativeCallValidation {
        let target_name = native_target_name(target);
        let Some(binding_key) = native_target_binding_key(target) else {
            return if is_reserved_std_native_target(&target_name) {
                NativeCallValidation::Invalid(format!(
                    "unknown built-in std native target {target_name}"
                ))
            } else {
                NativeCallValidation::External
            };
        };
        let Some(spec) = self.binding_spec(binding_key) else {
            return if is_reserved_std_native_target(binding_key)
                || is_reserved_std_native_target(&target_name)
            {
                NativeCallValidation::Invalid(format!(
                    "unknown built-in std native binding key {binding_key} for target {target_name}"
                ))
            } else {
                NativeCallValidation::External
            };
        };
        if !target.metadata.is_empty() {
            return NativeCallValidation::Invalid(
                "known std native target metadata is not supported".to_string(),
            );
        }

        if let Err(message) = validate_native_call_arg_count(spec.signature, arg_count) {
            return NativeCallValidation::Invalid(message);
        }

        validate_native_call_type_arg_refs(spec.signature, type_args)
            .map_or(NativeCallValidation::Known, NativeCallValidation::Invalid)
    }

    pub fn validate_native_dispatch_target<'a>(
        &self,
        target: &'a NativeTarget,
    ) -> NativeDispatchTarget<'a> {
        let target_name = native_target_name(target);
        let Some(binding_key) = native_target_binding_key(target) else {
            return if is_reserved_std_native_target(&target_name) {
                NativeDispatchTarget::Invalid(format!(
                    "{target_name} native call is missing artifact bindingKey"
                ))
            } else {
                NativeDispatchTarget::MissingExternalBinding { target_name }
            };
        };
        let Some(_spec) = self.binding_spec(binding_key) else {
            return if is_reserved_std_native_target(binding_key)
                || is_reserved_std_native_target(&target_name)
            {
                NativeDispatchTarget::Invalid(format!(
                    "unknown built-in std native binding key {binding_key} for target {target_name}"
                ))
            } else {
                NativeDispatchTarget::Resolved {
                    target_name,
                    binding_key,
                }
            };
        };
        if !target.metadata.is_empty() {
            return NativeDispatchTarget::Invalid(format!(
                "{target_name} call target metadata is not supported"
            ));
        }
        NativeDispatchTarget::Resolved {
            target_name,
            binding_key,
        }
    }
}

#[cfg(test)]
mod tests;
