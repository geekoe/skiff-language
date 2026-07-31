use std::collections::BTreeMap;
use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::{GatewayAdapterArg, GatewayAdapterKind, GatewayEntryKey};

/// Shared source-level dependency alias vectors.  Package and contract
/// dependencies deliberately consume this leaf owner instead of maintaining
/// separate lexical or reserved-word tables.
pub const DEPENDENCY_ALIAS_POSITIVE_VECTORS: &[&str] = &["a", "accounts", "a0", "a_B9"];
pub const DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS: &[&str] = &[
    "",
    "Accounts",
    "9accounts",
    "_accounts",
    "account-service",
    "account.service",
    "账户",
];
pub const DEPENDENCY_ALIAS_RESERVED_VECTORS: &[&str] = &[
    "package", "service", "std", "ext", "connect", "config", "root",
];

pub fn is_dependency_alias_lexically_valid(alias: &str) -> bool {
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub fn is_dependency_alias_reserved(alias: &str) -> bool {
    DEPENDENCY_ALIAS_RESERVED_VECTORS.contains(&alias)
}

pub fn is_dependency_alias_valid(alias: &str) -> bool {
    is_dependency_alias_lexically_valid(alias) && !is_dependency_alias_reserved(alias)
}

/// Service-only source manifest. A service source root remains a normal package
/// root; this DTO deliberately has no version, dependency or API surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceAuthoringKind {
    #[default]
    Service,
    Test,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpGatewayEntryAuthoring {
    pub method: String,
    pub path: String,
    pub kind: GatewayAdapterKind,
    pub handler: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<GatewayAdapterArg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSocketConnectAuthoring {
    pub handler: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<GatewayAdapterArg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSocketJsonRpcMethodAuthoring {
    pub method: String,
    pub handler: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<GatewayAdapterArg>,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HttpGatewayDocumentAuthoring {
    pub entries: BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>,
}

impl<'de> Deserialize<'de> for HttpGatewayDocumentAuthoring {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserialize_http_gateway_entry_map(deserializer).map(|entries| Self { entries })
    }
}

struct WebSocketJsonRpcMethodsVisitor;

impl<'de> Visitor<'de> for WebSocketJsonRpcMethodsVisitor {
    type Value = BTreeMap<GatewayEntryKey, WebSocketJsonRpcMethodAuthoring>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a mapping of unique WebSocket JSON-RPC gateway entry keys")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        while let Some((key, entry)) =
            access.next_entry::<GatewayEntryKey, WebSocketJsonRpcMethodAuthoring>()?
        {
            if entries.insert(key.clone(), entry).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate WebSocket JSON-RPC gateway entry key {key:?}"
                )));
            }
        }
        Ok(entries)
    }
}

fn deserialize_websocket_json_rpc_method_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<GatewayEntryKey, WebSocketJsonRpcMethodAuthoring>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_map(WebSocketJsonRpcMethodsVisitor)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSocketGatewayDocumentAuthoring {
    pub path: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub connect: Option<WebSocketConnectAuthoring>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "deserialize_websocket_json_rpc_method_map"
    )]
    pub json_rpc: BTreeMap<GatewayEntryKey, WebSocketJsonRpcMethodAuthoring>,
}

struct HttpGatewayEntriesVisitor;

impl<'de> Visitor<'de> for HttpGatewayEntriesVisitor {
    type Value = BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a mapping of unique HTTP gateway entry keys")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        while let Some((key, entry)) =
            access.next_entry::<GatewayEntryKey, HttpGatewayEntryAuthoring>()?
        {
            if entries.insert(key.clone(), entry).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate HTTP gateway entry key {key:?}"
                )));
            }
        }
        Ok(entries)
    }
}

fn deserialize_http_gateway_entry_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_map(HttpGatewayEntriesVisitor)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceManifestAuthoring {
    pub id: String,
    #[serde(default)]
    pub kind: ServiceAuthoringKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_calls: Vec<String>,
}

/// One runtime-config source file. Its root is the canonical Package ID map;
/// a service's own package uses the same key space as every dependency.
///
/// Package ID syntax is validated by compiler input. This shared wire owner
/// enforces the structural `/` separator so retired profile wrappers cannot
/// silently become package keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct RuntimeConfigSourceAuthoring {
    packages: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

impl RuntimeConfigSourceAuthoring {
    pub fn packages(&self) -> &BTreeMap<String, BTreeMap<String, serde_json::Value>> {
        &self.packages
    }

    pub fn into_packages(self) -> BTreeMap<String, BTreeMap<String, serde_json::Value>> {
        self.packages
    }
}

impl<'de> Deserialize<'de> for RuntimeConfigSourceAuthoring {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let packages =
            BTreeMap::<String, BTreeMap<String, serde_json::Value>>::deserialize(deserializer)?;
        if let Some(package_id) = packages.keys().find(|package_id| {
            let Some((authority, local_path)) = package_id.split_once('/') else {
                return true;
            };
            authority.is_empty() || local_path.is_empty()
        }) {
            return Err(serde::de::Error::custom(format!(
                "runtime config root key {package_id:?} must be a canonical Package ID"
            )));
        }
        Ok(Self { packages })
    }
}

#[cfg(test)]
mod tests;
