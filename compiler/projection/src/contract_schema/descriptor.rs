use std::collections::BTreeMap;

use serde::Serialize;
use skiff_artifact_model::{LiteralIr, ServiceSymbolRef, TypeRefIr};

/// Strongly-typed runtime type descriptor: the `{ "kind": ... }` object the
/// contract layer projects for operation return types, parameter types and db
/// field types. This is the producer-side mirror of what the runtime's
/// `RuntimeTypeDescriptor` reads back out of the serialized artifact (the
/// runtime keeps it as `serde_json::Value`, so this type only exists on the
/// compiler projection side and is serialized at the artifact boundary).
///
/// Field order within each variant matches the former `json!` construction so
/// serialized bytes are unchanged. `record`/builtin `fields` use a BTreeMap,
/// matching the prior `serde_json::Map` whose keys were already inserted in
/// sorted order.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeTypeDescriptorIr {
    Builtin {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<Vec<RuntimeTypeDescriptorIr>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fields: Option<BTreeMap<String, RuntimeTypeDescriptorIr>>,
    },
    Nullable {
        inner: Box<RuntimeTypeDescriptorIr>,
    },
    Union {
        items: Vec<RuntimeTypeDescriptorIr>,
    },
    Record {
        fields: BTreeMap<String, RuntimeTypeDescriptorIr>,
    },
    Literal {
        value: LiteralIr,
    },
    Representation {
        name: String,
        representation: Box<RuntimeTypeDescriptorIr>,
        #[serde(skip_serializing_if = "Option::is_none")]
        discriminator: Option<String>,
    },
}

impl RuntimeTypeDescriptorIr {
    /// `{ "kind": "builtin", "name": <name> }`
    pub fn named(name: impl Into<String>) -> Self {
        RuntimeTypeDescriptorIr::Builtin {
            name: name.into(),
            args: None,
            fields: None,
        }
    }

    /// `{ "kind": "builtin", "name": <name>, "fields": <fields> }`
    pub fn named_with_fields(
        name: impl Into<String>,
        fields: BTreeMap<String, RuntimeTypeDescriptorIr>,
    ) -> Self {
        RuntimeTypeDescriptorIr::Builtin {
            name: name.into(),
            args: None,
            fields: Some(fields),
        }
    }

    /// `{ "kind": "literal", "value": { "kind": "string", "value": <value> } }`
    pub fn literal_string(value: impl Into<String>) -> Self {
        RuntimeTypeDescriptorIr::Literal {
            value: LiteralIr::String {
                value: value.into(),
            },
        }
    }

    pub fn nullable(inner: RuntimeTypeDescriptorIr) -> Self {
        RuntimeTypeDescriptorIr::Nullable {
            inner: Box::new(inner),
        }
    }

    pub fn to_type_ref_for_service_unit(&self) -> TypeRefIr {
        self.try_as_type_ref()
            .unwrap_or_else(|| TypeRefIr::ServiceSymbol {
                symbol: service_symbol(self.fallback_symbol_name()),
            })
    }

    fn try_as_type_ref(&self) -> Option<TypeRefIr> {
        match self {
            RuntimeTypeDescriptorIr::Builtin { name, args, fields } => {
                if fields.is_some() {
                    return Some(TypeRefIr::ServiceSymbol {
                        symbol: service_symbol(name),
                    });
                }
                Some(TypeRefIr::Native {
                    name: name.clone(),
                    args: args
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .map(RuntimeTypeDescriptorIr::try_as_type_ref)
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            RuntimeTypeDescriptorIr::Nullable { inner } => Some(TypeRefIr::Nullable {
                inner: Box::new(inner.try_as_type_ref()?),
            }),
            RuntimeTypeDescriptorIr::Union { items } => Some(TypeRefIr::Union {
                items: items
                    .iter()
                    .map(RuntimeTypeDescriptorIr::try_as_type_ref)
                    .collect::<Option<Vec<_>>>()?,
            }),
            RuntimeTypeDescriptorIr::Record { fields } => Some(TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| Some((name.clone(), ty.try_as_type_ref()?)))
                    .collect::<Option<BTreeMap<_, _>>>()?,
            }),
            RuntimeTypeDescriptorIr::Literal { value } => Some(TypeRefIr::Literal {
                value: value.clone(),
            }),
            RuntimeTypeDescriptorIr::Representation { .. } => None,
        }
    }

    fn fallback_symbol_name(&self) -> &str {
        match self {
            RuntimeTypeDescriptorIr::Builtin { name, .. }
            | RuntimeTypeDescriptorIr::Representation { name, .. } => name,
            RuntimeTypeDescriptorIr::Nullable { .. }
            | RuntimeTypeDescriptorIr::Union { .. }
            | RuntimeTypeDescriptorIr::Record { .. }
            | RuntimeTypeDescriptorIr::Literal { .. } => "Json",
        }
    }
}

fn service_symbol(name: &str) -> ServiceSymbolRef {
    ServiceSymbolRef {
        module_path: String::new(),
        symbol: name.to_string(),
    }
}
