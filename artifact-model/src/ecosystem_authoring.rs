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
mod tests {
    use super::*;

    #[test]
    fn dependency_alias_vectors_have_one_shared_leaf_owner() {
        for alias in DEPENDENCY_ALIAS_POSITIVE_VECTORS {
            assert!(is_dependency_alias_lexically_valid(alias), "{alias}");
            assert!(!is_dependency_alias_reserved(alias), "{alias}");
            assert!(is_dependency_alias_valid(alias), "{alias}");
        }
        for alias in DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS {
            assert!(!is_dependency_alias_lexically_valid(alias), "{alias}");
            assert!(!is_dependency_alias_valid(alias), "{alias}");
        }
        for alias in DEPENDENCY_ALIAS_RESERVED_VECTORS {
            assert!(is_dependency_alias_lexically_valid(alias), "{alias}");
            assert!(is_dependency_alias_reserved(alias), "{alias}");
            assert!(!is_dependency_alias_valid(alias), "{alias}");
        }
    }

    #[test]
    fn service_manifest_missing_and_empty_service_calls_are_equivalent() {
        let missing =
            serde_yaml::from_str::<ServiceManifestAuthoring>("id: example.com/users\n").unwrap();
        let empty = serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/users\nserviceCalls: []\n",
        )
        .unwrap();
        assert_eq!(missing, empty);
        assert!(missing.service_calls.is_empty());
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            serde_json::json!({
                "id": "example.com/users",
                "kind": "service"
            })
        );

        let unvalidated = serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/users\nserviceCalls:\n  - users.get\n  - not validated here\n",
        )
        .unwrap();
        assert_eq!(
            unvalidated.service_calls,
            vec!["users.get".to_string(), "not validated here".to_string()]
        );
        assert_eq!(
            serde_json::to_value(&unvalidated).unwrap()["serviceCalls"],
            serde_json::json!(["users.get", "not validated here"])
        );

        assert!(serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/users\nserviceCallRoots: []\n"
        )
        .is_err());
    }

    #[test]
    fn service_manifest_rejects_inline_external_fields() {
        for field in ["http: {}", "websocket: { path: /chat }", "timeout: 1000"] {
            let source = format!("id: example.com/users\n{field}\n");
            assert!(
                serde_yaml::from_str::<ServiceManifestAuthoring>(&source).is_err(),
                "{field} must not remain in service.yml"
            );
        }
    }

    #[test]
    fn runtime_config_source_is_a_package_id_root_map_without_profile_wrappers() {
        let source = serde_yaml::from_str::<RuntimeConfigSourceAuthoring>(
            r#"
agine.ai/api:
  model: default
skiff.run/http-session:
  cookieName: agine_session
  maxAgeSeconds: 2592000
"#,
        )
        .unwrap();
        assert_eq!(
            source.packages()["skiff.run/http-session"]["cookieName"],
            serde_json::json!("agine_session")
        );
        assert_eq!(
            serde_json::to_value(source).unwrap(),
            serde_json::json!({
                "agine.ai/api": { "model": "default" },
                "skiff.run/http-session": {
                    "cookieName": "agine_session",
                    "maxAgeSeconds": 2592000
                }
            })
        );

        for retired in [
            "config: {}\n",
            "secrets: {}\n",
            "state: {}\n",
            "resources: {}\n",
            "timeout: {}\n",
            "quota: {}\n",
            "principal: {}\n",
        ] {
            assert!(
                serde_yaml::from_str::<RuntimeConfigSourceAuthoring>(retired).is_err(),
                "{retired:?} unexpectedly survived as a runtime config root"
            );
        }
    }

    #[test]
    fn http_document_decodes_named_entries_in_canonical_key_order() {
        let document = serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(
            r#"
zRaw:
  method: GET
  path: /raw
  kind: rawHttp
  handler: handlers.raw
createUser:
  method: POST
  path: /users
  kind: typedJson
  handler: users.create
  guard: users.guard
  pre: users.prepare
  adapterArgs:
    - param: body
      source: { kind: http.body }
    - param: context
      source: { kind: http.context }
"#,
        )
        .unwrap();
        assert_eq!(
            document
                .entries
                .keys()
                .map(GatewayEntryKey::as_str)
                .collect::<Vec<_>>(),
            vec!["createUser", "zRaw"]
        );
        let encoded = serde_json::to_string(&document).unwrap();
        assert!(!encoded.contains("\"host\""));
        assert!(encoded.find("createUser").unwrap() < encoded.find("zRaw").unwrap());
        assert_eq!(
            serde_json::from_str::<HttpGatewayDocumentAuthoring>(&encoded).unwrap(),
            document
        );
        assert!(serde_yaml::from_str::<HttpGatewayDocumentAuthoring>("{}")
            .unwrap()
            .entries
            .is_empty());
        for invalid in ["null", "value", "[]", "http: {}"] {
            assert!(
                serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(invalid).is_err(),
                "{invalid:?} unexpectedly decoded"
            );
        }
    }

    #[test]
    fn http_document_rejects_duplicate_keys_and_recursive_unknown_fields() {
        let duplicate = r#"
createUser:
  method: POST
  path: /users
  kind: typedJson
  handler: users.create
createUser:
  method: PUT
  path: /users
  kind: typedJson
  handler: users.replace
"#;
        assert!(
            serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate HTTP gateway entry key")
        );

        for invalid in [
            "unknown: true",
            "operation: createUser",
            "handlerArgs: []",
            "id: duplicate",
        ] {
            let source = format!(
                "createUser:\n  method: POST\n  path: /users\n  kind: typedJson\n  handler: users.create\n  {invalid}\n"
            );
            assert!(
                serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(&source).is_err(),
                "{invalid}"
            );
        }
        let legacy_host = r#"
createUser:
  host: api.example.com
  method: POST
  path: /users
  kind: typedJson
  handler: users.create
"#;
        assert!(
            serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(legacy_host)
                .unwrap_err()
                .to_string()
                .contains("unknown field `host`")
        );

        let unknown_source_field = r#"
createUser:
  method: POST
  path: /users
  kind: typedJson
  handler: users.create
  adapterArgs:
    - param: body
      source: { kind: http.body, field: nested }
"#;
        assert!(
            serde_yaml::from_str::<HttpGatewayDocumentAuthoring>(unknown_source_field).is_err()
        );
    }

    #[test]
    fn websocket_document_is_one_strict_entry_with_declared_json_rpc_methods() {
        let path_only = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(
            r#"
path: /chat
"#,
        )
        .unwrap();
        assert_eq!(path_only.path, "/chat");
        assert!(path_only.connect.is_none());
        assert!(path_only.json_rpc.is_empty());

        let with_connect = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(
            r#"
path: /chat
connect:
  handler: handlers.connect
  adapterArgs:
    - param: request
      source: { kind: websocket.connectRequest }
    - param: connectionId
      source: { kind: websocket.connectionId }
jsonRpc:
  getStatus:
    method: status.get
    handler: handlers.getStatus
    adapterArgs:
      - param: input
        source: { kind: websocket.jsonRpcParams }
"#,
        )
        .unwrap();
        assert_eq!(with_connect.connect.as_ref().unwrap().adapter_args.len(), 2);
        assert_eq!(
            with_connect.json_rpc[&GatewayEntryKey::parse("getStatus").unwrap()].method,
            "status.get"
        );
        assert_eq!(
            serde_json::from_value::<WebSocketGatewayDocumentAuthoring>(
                serde_json::to_value(&with_connect).unwrap()
            )
            .unwrap(),
            with_connect
        );
    }

    #[test]
    fn websocket_document_rejects_null_collection_legacy_and_duplicate_shapes() {
        for (label, source) in [
            ("empty file", ""),
            ("null", "null"),
            ("scalar", "chat"),
            ("list", "[]"),
            (
                "named multi-entry map",
                "{ first: { path: /one }, second: { path: /two } }",
            ),
            ("missing path", "{}"),
            ("null connect", "{ path: /chat, connect: null }"),
            ("missing handler", "{ path: /chat, connect: {} }"),
            ("author id", "{ id: author, path: /chat }"),
            ("host", "{ host: chat.example.com, path: /chat }"),
            ("wrapper", "{ websocket: { path: /chat } }"),
            ("routes", "{ path: /chat, routes: [] }"),
            ("operation", "{ path: /chat, operation: receive }"),
            ("receive", "{ path: /chat, receive: handlers.receive }"),
            ("message", "{ path: /chat, message: handlers.message }"),
            ("context", "{ path: /chat, context: Context }"),
            ("unknown", "{ path: /chat, unknown: true }"),
        ] {
            assert!(
                serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(source).is_err(),
                "{label} unexpectedly decoded"
            );
        }

        for duplicate in [
            "path: /one\npath: /two\n",
            "path: /chat\nconnect:\n  handler: one.connect\n  handler: two.connect\n",
            "path: /chat\njsonRpc:\n  status:\n    method: status.get\n    handler: one.status\n  status:\n    method: status.set\n    handler: two.status\n",
            "path: /chat\njsonRpc:\n  status:\n    method: status.get\n    handler: one.status\n    handler: two.status\n",
        ] {
            assert!(
                serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(duplicate).is_err(),
                "duplicate field unexpectedly decoded"
            );
        }
    }
}
