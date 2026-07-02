use skiff_artifact_model::{NativeSignatureDef, NativeTarget};

use super::NativeRequiredContext;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NativeBindingKey(&'static str);

impl NativeBindingKey {
    pub fn from_static(value: &'static str) -> Self {
        Self(value)
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NativeBindingSpec {
    pub key: NativeBindingKey,
    pub signature: &'static NativeSignatureDef,
    pub required_context: NativeRequiredContext,
}

impl NativeBindingSpec {
    pub fn from_signature(signature: &'static NativeSignatureDef) -> Option<Self> {
        let required_context = NativeRequiredContext::for_binding_key(signature.binding_key)?;
        Some(Self {
            key: NativeBindingKey::from_static(signature.binding_key),
            signature,
            required_context,
        })
    }
}

pub fn native_target_binding_key(target: &NativeTarget) -> Option<&str> {
    target.binding_key.as_deref().filter(|key| !key.is_empty())
}

pub fn native_target_name(target: &NativeTarget) -> String {
    if target.namespace.is_empty() {
        target.symbol.clone()
    } else {
        format!("{}.{}", target.namespace, target.symbol)
    }
}
