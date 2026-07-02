use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const RUNTIME_MANIFEST_SCHEMA_VERSION: &str = "skiff-runtime-manifest-v1";
pub const DEFAULT_SERVICE_ID: &str = "skiff-dev";
pub const RUNTIME_OPERATION_MODE_UNARY: &str = "unary";
pub const RUNTIME_OPERATION_MODE_SERVER_STREAM: &str = "serverStream";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactOperation {
    pub operation: String,
    pub target: Option<String>,
    pub function: String,
    pub parameters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkiffRuntimeManifest {
    pub schema_version: String,
    pub service: RuntimeServiceManifest,
    pub operations: Vec<RuntimeOperationManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<RuntimeGatewayManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<RuntimeTimeoutManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeServiceManifest {
    pub id: String,
    pub revision_id: String,
    pub protocol_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<RuntimeServiceAccessManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeServiceAccessManifest {
    pub visibility: RuntimeServiceVisibility,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_role: Option<RuntimeServiceOrganizationRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeServiceVisibility {
    Public,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeServiceOrganizationRole {
    Viewer,
    Maintainer,
    Owner,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOperationManifest {
    pub operation: String,
    pub operation_abi_id: String,
    pub target: String,
    pub mode: String,
    pub parameters: Vec<RuntimeOperationParameter>,
    pub response: JsonSchema,
    pub service_protocol_identity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeOperationParameter {
    pub name: String,
    pub schema: JsonSchema,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeGatewayManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<RuntimeHttpGatewayManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket: Option<RuntimeWebSocketGatewayManifest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHttpGatewayManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<RuntimeHttpRawGatewayManifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<RuntimeHttpRouteGatewayManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHttpRawGatewayManifest {
    pub operation: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHttpRouteGatewayManifest {
    pub method: String,
    pub path: String,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_abi_id: Option<String>,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler: Option<RuntimeHttpRouteHandlerManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<RuntimeHttpRouteAdapterManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typed: Option<RuntimeHttpRouteTypedManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeHttpRouteHandlerManifest {
    ServiceFunction {
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        package_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
        symbol_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHttpRouteTypedManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<RuntimeHttpRouteTypedBodyManifest>,
    pub response: RuntimeHttpRouteTypedResponseManifest,
    pub ingress_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<RuntimeHttpRouteAdapterManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHttpRouteAdapterManifest {
    pub kind: RuntimeHttpRouteAdapterKind,
    pub handler: RuntimeHttpRouteAdapterCallableManifest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<RuntimeHttpRouteAdapterCallableManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre: Option<RuntimeHttpRouteAdapterCallableManifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<RuntimeGatewayAdapterArgManifest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeHttpRouteAdapterKind {
    TypedJson,
    RawHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeHttpRouteAdapterCallableManifest {
    ServiceFunction {
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        package_id: String,
        symbol_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeGatewayAdapterArgManifest {
    pub param: String,
    pub source: RuntimeGatewayAdapterSourceManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RuntimeGatewayAdapterSourceManifest {
    #[serde(rename = "http.request")]
    HttpRequest,
    #[serde(rename = "http.body")]
    HttpBody,
    #[serde(rename = "http.context")]
    HttpContext,
    #[serde(rename = "websocket.connectRequest")]
    WebSocketConnectRequest,
    #[serde(rename = "websocket.receiveEvent")]
    WebSocketReceiveEvent,
    #[serde(rename = "websocket.connection")]
    WebSocketConnection,
    #[serde(rename = "websocket.connectionContext")]
    WebSocketConnectionContext,
    #[serde(rename = "websocket.message")]
    WebSocketMessage,
    #[serde(rename = "websocket.messageBody")]
    WebSocketMessageBody,
    #[serde(rename = "websocket.connectionId")]
    WebSocketConnectionId,
    #[serde(rename = "websocket.businessIdentity")]
    WebSocketBusinessIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHttpRouteTypedBodyManifest {
    pub schema: JsonSchema,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHttpRouteTypedResponseManifest {
    pub schema: JsonSchema,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeWebSocketGatewayManifest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<JsonSchema>,
    pub context_expectation: RuntimeWebSocketContextExpectationManifest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect: Option<RuntimeWebSocketOperationManifest>,
    pub receive: RuntimeWebSocketOperationManifest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_entry_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeWebSocketContextExpectationManifest {
    Null,
    Typed {
        connect_operation_abi_id: String,
        context_type_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeWebSocketOperationManifest {
    pub operation: String,
    pub operation_abi_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<RuntimeGatewayAdapterArgManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_operation_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_protocol_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_entry_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeTimeoutManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_ms: Option<u64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub methods: BTreeMap<String, u64>,
}

/// Value of a JSON Schema `additionalProperties` keyword.
///
/// Either a boolean (open/closed objects) or a nested schema (the value schema
/// of a `map`). Modelled as a typed enum so the schema tree never holds a
/// `serde_json::Value`.
#[derive(Debug, Clone, PartialEq)]
pub enum AdditionalProperties {
    Bool(bool),
    Schema(Box<JsonSchema>),
}

/// A typed, recursive JSON Schema node.
///
/// The body is fully typed: every child schema is a `Box<JsonSchema>`,
/// `Vec<JsonSchema>`, or `BTreeMap<String, JsonSchema>`, never a
/// `serde_json::Value`. The field vocabulary is a closed, known set (the JSON
/// Schema subset actually emitted plus the `xSkiff*` extensions). `Value` only
/// appears at the `Serialize`/`Deserialize` boundary (artifact emission), never
/// as an internal protocol.
///
/// Serialization is byte-identical to the previous `BTreeMap<String, Value>`
/// `#[serde(flatten)]` representation: fields are emitted in alphabetical key
/// order (the BTreeMap iteration order), each only when set.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsonSchema {
    /// `$ref`
    pub reference: Option<String>,
    /// `additionalProperties`
    pub additional_properties: Option<AdditionalProperties>,
    /// `contentEncoding`
    pub content_encoding: Option<String>,
    /// `enum`
    pub enum_values: Option<Vec<String>>,
    /// `format`
    pub format: Option<String>,
    /// `items`
    pub items: Option<Box<JsonSchema>>,
    /// `nullable`
    pub nullable: Option<bool>,
    /// `oneOf`
    pub one_of: Option<Vec<JsonSchema>>,
    /// `properties`
    pub properties: Option<BTreeMap<String, JsonSchema>>,
    /// `required`
    pub required: Option<Vec<String>>,
    /// `type`
    pub schema_type: Option<String>,
    /// `uniqueItems`
    pub unique_items: Option<bool>,
    /// `xSkiffAlias`
    pub x_skiff_alias: Option<String>,
    /// `xSkiffMapKeySchema`
    pub x_skiff_map_key_schema: Option<Box<JsonSchema>>,
    /// `xSkiffMapKeySymbol`
    pub x_skiff_map_key_symbol: Option<String>,
    /// `xSkiffPackage`
    pub x_skiff_package: Option<String>,
    /// `xSkiffPreludeIdentity`
    pub x_skiff_prelude_identity: Option<String>,
    /// `xSkiffSchemaIdentity`
    pub x_skiff_schema_identity: Option<String>,
    /// `xSkiffSymbol`
    pub x_skiff_symbol: Option<String>,
    /// `xSkiffUnionBranch`
    pub x_skiff_union_branch: Option<String>,
    /// `xSkiffUnionDiscriminator`
    pub x_skiff_union_discriminator: Option<String>,
}

impl JsonSchema {
    pub fn any() -> Self {
        Self::typed("any")
    }

    pub fn typed(schema_type: &str) -> Self {
        Self {
            schema_type: Some(schema_type.to_string()),
            ..Self::default()
        }
    }

    pub fn string() -> Self {
        Self::typed("string")
    }

    pub fn number() -> Self {
        Self::typed("number")
    }

    pub fn integer() -> Self {
        Self::typed("integer")
    }

    pub fn boolean() -> Self {
        Self::typed("boolean")
    }

    pub fn null() -> Self {
        Self::typed("null")
    }

    pub fn reference(reference: &str) -> Self {
        Self {
            reference: Some(reference.to_string()),
            ..Self::default()
        }
    }

    pub fn string_enum(values: Vec<String>) -> Self {
        let mut schema = Self::string();
        schema.enum_values = Some(values);
        schema
    }

    pub fn one_of(schemas: Vec<JsonSchema>) -> Self {
        Self {
            one_of: Some(schemas),
            ..Self::default()
        }
    }

    pub fn array(items: JsonSchema) -> Self {
        let mut schema = Self::typed("array");
        schema.items = Some(Box::new(items));
        schema
    }

    pub fn set(items: JsonSchema) -> Self {
        let mut schema = Self::array(items);
        schema.unique_items = Some(true);
        schema
    }

    pub fn map(values: JsonSchema) -> Self {
        let mut schema = Self::typed("object");
        schema.additional_properties = Some(AdditionalProperties::Schema(Box::new(values)));
        schema
    }

    pub fn object(
        properties: BTreeMap<String, JsonSchema>,
        required: Vec<String>,
        additional_properties: bool,
    ) -> Self {
        let mut schema = Self::typed("object");
        schema.properties = Some(properties);
        schema.additional_properties = Some(AdditionalProperties::Bool(additional_properties));
        if !required.is_empty() {
            schema.required = Some(required);
        }
        schema
    }

    pub fn nullable(mut self) -> Self {
        self.nullable = Some(true);
        self
    }

    pub fn is_nullable(&self) -> bool {
        self.nullable == Some(true)
    }

    // --- typed setters (replace the former `with_field(Value)` API) ---

    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    pub fn with_content_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.content_encoding = Some(encoding.into());
        self
    }

    pub fn with_additional_properties(mut self, additional_properties: bool) -> Self {
        self.additional_properties = Some(AdditionalProperties::Bool(additional_properties));
        self
    }

    pub fn with_required(mut self, required: Vec<String>) -> Self {
        self.required = Some(required);
        self
    }

    pub fn with_x_skiff_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.x_skiff_symbol = Some(symbol.into());
        self
    }

    pub fn with_x_skiff_alias(mut self, alias: impl Into<String>) -> Self {
        self.x_skiff_alias = Some(alias.into());
        self
    }

    pub fn with_x_skiff_prelude_identity(mut self, identity: impl Into<String>) -> Self {
        self.x_skiff_prelude_identity = Some(identity.into());
        self
    }

    pub fn with_x_skiff_schema_identity(mut self, identity: impl Into<String>) -> Self {
        self.x_skiff_schema_identity = Some(identity.into());
        self
    }

    pub fn with_x_skiff_package(mut self, package: impl Into<String>) -> Self {
        self.x_skiff_package = Some(package.into());
        self
    }

    pub fn with_x_skiff_union_branch(mut self, branch: impl Into<String>) -> Self {
        self.x_skiff_union_branch = Some(branch.into());
        self
    }

    pub fn with_x_skiff_union_discriminator(mut self, discriminator: impl Into<String>) -> Self {
        self.x_skiff_union_discriminator = Some(discriminator.into());
        self
    }

    pub fn with_x_skiff_map_key_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.x_skiff_map_key_symbol = Some(symbol.into());
        self
    }

    pub fn with_x_skiff_map_key_schema(mut self, schema: JsonSchema) -> Self {
        self.x_skiff_map_key_schema = Some(Box::new(schema));
        self
    }

    /// Read accessor for the `type` keyword, kept for tests that previously read
    /// `schema.fields["type"]`.
    pub fn schema_type(&self) -> Option<&str> {
        self.schema_type.as_deref()
    }

    // --- typed accessors for symbol rewriting (replaces Value-map traversal) ---

    pub fn reference_mut(&mut self) -> &mut Option<String> {
        &mut self.reference
    }

    pub fn x_skiff_alias_mut(&mut self) -> &mut Option<String> {
        &mut self.x_skiff_alias
    }

    pub fn x_skiff_map_key_symbol_mut(&mut self) -> &mut Option<String> {
        &mut self.x_skiff_map_key_symbol
    }

    pub fn x_skiff_symbol_mut(&mut self) -> &mut Option<String> {
        &mut self.x_skiff_symbol
    }

    /// Mutable references to every directly nested child schema node, for
    /// recursive in-place traversal (e.g. symbol rewriting).
    pub fn child_schemas_mut(&mut self) -> Vec<&mut JsonSchema> {
        let mut children: Vec<&mut JsonSchema> = Vec::new();
        if let Some(items) = self.items.as_mut() {
            children.push(items.as_mut());
        }
        if let Some(one_of) = self.one_of.as_mut() {
            children.extend(one_of.iter_mut());
        }
        if let Some(properties) = self.properties.as_mut() {
            children.extend(properties.values_mut());
        }
        if let Some(AdditionalProperties::Schema(schema)) = self.additional_properties.as_mut() {
            children.push(schema.as_mut());
        }
        if let Some(schema) = self.x_skiff_map_key_schema.as_mut() {
            children.push(schema.as_mut());
        }
        children
    }
}

impl Serialize for AdditionalProperties {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            AdditionalProperties::Bool(value) => serializer.serialize_bool(*value),
            AdditionalProperties::Schema(schema) => schema.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for AdditionalProperties {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::Bool(flag) => Ok(AdditionalProperties::Bool(flag)),
            other => {
                let schema = JsonSchema::deserialize(other).map_err(serde::de::Error::custom)?;
                Ok(AdditionalProperties::Schema(Box::new(schema)))
            }
        }
    }
}

impl Serialize for JsonSchema {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        // Emit fields in alphabetical key order (the BTreeMap iteration order
        // of the prior `#[serde(flatten)] BTreeMap<String, Value>`), each only
        // when set. This keeps artifact bytes identical.
        let mut map = serializer.serialize_map(None)?;
        if let Some(reference) = &self.reference {
            map.serialize_entry("$ref", reference)?;
        }
        if let Some(additional_properties) = &self.additional_properties {
            map.serialize_entry("additionalProperties", additional_properties)?;
        }
        if let Some(content_encoding) = &self.content_encoding {
            map.serialize_entry("contentEncoding", content_encoding)?;
        }
        if let Some(enum_values) = &self.enum_values {
            map.serialize_entry("enum", enum_values)?;
        }
        if let Some(format) = &self.format {
            map.serialize_entry("format", format)?;
        }
        if let Some(items) = &self.items {
            map.serialize_entry("items", items)?;
        }
        if let Some(nullable) = &self.nullable {
            map.serialize_entry("nullable", nullable)?;
        }
        if let Some(one_of) = &self.one_of {
            map.serialize_entry("oneOf", one_of)?;
        }
        if let Some(properties) = &self.properties {
            map.serialize_entry("properties", properties)?;
        }
        if let Some(required) = &self.required {
            map.serialize_entry("required", required)?;
        }
        if let Some(schema_type) = &self.schema_type {
            map.serialize_entry("type", schema_type)?;
        }
        if let Some(unique_items) = &self.unique_items {
            map.serialize_entry("uniqueItems", unique_items)?;
        }
        if let Some(x_skiff_alias) = &self.x_skiff_alias {
            map.serialize_entry("xSkiffAlias", x_skiff_alias)?;
        }
        if let Some(x_skiff_map_key_schema) = &self.x_skiff_map_key_schema {
            map.serialize_entry("xSkiffMapKeySchema", x_skiff_map_key_schema)?;
        }
        if let Some(x_skiff_map_key_symbol) = &self.x_skiff_map_key_symbol {
            map.serialize_entry("xSkiffMapKeySymbol", x_skiff_map_key_symbol)?;
        }
        if let Some(x_skiff_package) = &self.x_skiff_package {
            map.serialize_entry("xSkiffPackage", x_skiff_package)?;
        }
        if let Some(x_skiff_prelude_identity) = &self.x_skiff_prelude_identity {
            map.serialize_entry("xSkiffPreludeIdentity", x_skiff_prelude_identity)?;
        }
        if let Some(x_skiff_schema_identity) = &self.x_skiff_schema_identity {
            map.serialize_entry("xSkiffSchemaIdentity", x_skiff_schema_identity)?;
        }
        if let Some(x_skiff_symbol) = &self.x_skiff_symbol {
            map.serialize_entry("xSkiffSymbol", x_skiff_symbol)?;
        }
        if let Some(x_skiff_union_branch) = &self.x_skiff_union_branch {
            map.serialize_entry("xSkiffUnionBranch", x_skiff_union_branch)?;
        }
        if let Some(x_skiff_union_discriminator) = &self.x_skiff_union_discriminator {
            map.serialize_entry("xSkiffUnionDiscriminator", x_skiff_union_discriminator)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for JsonSchema {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(rename = "$ref", default)]
            reference: Option<String>,
            #[serde(rename = "additionalProperties", default)]
            additional_properties: Option<AdditionalProperties>,
            #[serde(rename = "contentEncoding", default)]
            content_encoding: Option<String>,
            #[serde(rename = "enum", default)]
            enum_values: Option<Vec<String>>,
            #[serde(default)]
            format: Option<String>,
            #[serde(default)]
            items: Option<Box<JsonSchema>>,
            #[serde(default)]
            nullable: Option<bool>,
            #[serde(rename = "oneOf", default)]
            one_of: Option<Vec<JsonSchema>>,
            #[serde(default)]
            properties: Option<BTreeMap<String, JsonSchema>>,
            #[serde(default)]
            required: Option<Vec<String>>,
            #[serde(rename = "type", default)]
            schema_type: Option<String>,
            #[serde(rename = "uniqueItems", default)]
            unique_items: Option<bool>,
            #[serde(rename = "xSkiffAlias", default)]
            x_skiff_alias: Option<String>,
            #[serde(rename = "xSkiffMapKeySchema", default)]
            x_skiff_map_key_schema: Option<Box<JsonSchema>>,
            #[serde(rename = "xSkiffMapKeySymbol", default)]
            x_skiff_map_key_symbol: Option<String>,
            #[serde(rename = "xSkiffPackage", default)]
            x_skiff_package: Option<String>,
            #[serde(rename = "xSkiffPreludeIdentity", default)]
            x_skiff_prelude_identity: Option<String>,
            #[serde(rename = "xSkiffSchemaIdentity", default)]
            x_skiff_schema_identity: Option<String>,
            #[serde(rename = "xSkiffSymbol", default)]
            x_skiff_symbol: Option<String>,
            #[serde(rename = "xSkiffUnionBranch", default)]
            x_skiff_union_branch: Option<String>,
            #[serde(rename = "xSkiffUnionDiscriminator", default)]
            x_skiff_union_discriminator: Option<String>,
        }

        let raw = Raw::deserialize(deserializer)?;
        Ok(JsonSchema {
            reference: raw.reference,
            additional_properties: raw.additional_properties,
            content_encoding: raw.content_encoding,
            enum_values: raw.enum_values,
            format: raw.format,
            items: raw.items,
            nullable: raw.nullable,
            one_of: raw.one_of,
            properties: raw.properties,
            required: raw.required,
            schema_type: raw.schema_type,
            unique_items: raw.unique_items,
            x_skiff_alias: raw.x_skiff_alias,
            x_skiff_map_key_schema: raw.x_skiff_map_key_schema,
            x_skiff_map_key_symbol: raw.x_skiff_map_key_symbol,
            x_skiff_package: raw.x_skiff_package,
            x_skiff_prelude_identity: raw.x_skiff_prelude_identity,
            x_skiff_schema_identity: raw.x_skiff_schema_identity,
            x_skiff_symbol: raw.x_skiff_symbol,
            x_skiff_union_branch: raw.x_skiff_union_branch,
            x_skiff_union_discriminator: raw.x_skiff_union_discriminator,
        })
    }
}

pub fn revision_id(
    service: &RuntimeServiceManifest,
    source: &str,
    operations: &[ArtifactOperation],
) -> String {
    manifest_hash(service, source, operations)
}

pub fn protocol_identity_from_canonical_schema(canonical_schema_json: &str) -> String {
    format!(
        "skiff-protocol-v1:sha256:{}",
        hex::encode(Sha256::digest(canonical_schema_json.as_bytes()))
    )
}

pub fn http_ingress_identity(
    service_id: &str,
    method: &str,
    path: &str,
    body: Option<&JsonSchema>,
    response: &JsonSchema,
) -> String {
    let canonical = serde_json::json!({
        "identitySchema": "skiff-http-ingress-identity-v1",
        "serviceId": service_id,
        "method": method,
        "path": path,
        "body": body,
        "response": response,
    });
    format!(
        "skiff-http-ingress-v1:sha256:{}",
        hex::encode(Sha256::digest(
            serde_json::to_vec(&canonical)
                .expect("HTTP ingress identity value must serialize")
                .as_slice()
        ))
    )
}

fn manifest_hash(
    service: &RuntimeServiceManifest,
    source: &str,
    operations: &[ArtifactOperation],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"skiff-runtime-manifest-v1\0");
    hasher.update(service.id.as_bytes());
    if let Some(access) = &service.access {
        hasher.update(b"\0access\0");
        hasher.update(
            serde_json::to_vec(access)
                .expect("service access manifest must serialize")
                .as_slice(),
        );
    }
    hasher.update(b"\0source\0");
    hasher.update(source.as_bytes());
    hasher.update(b"\0operations\0");
    for operation in operations {
        hasher.update(operation.operation.as_bytes());
        hasher.update(b"\0");
        let target = operation.target.clone().unwrap_or_default();
        hasher.update(target.as_bytes());
        hasher.update(b"\0");
        hasher.update(operation.function.as_bytes());
        hasher.update(b"\0");
        for parameter in &operation.parameters {
            hasher.update(parameter.as_bytes());
            hasher.update(b"\0");
        }
        hasher.update(b"\0operation-end\0");
    }
    hex::encode(hasher.finalize())
}
