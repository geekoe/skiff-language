use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, OnceLock},
};

use crate::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryErrorContract,
    BoundaryStreamContract, ContractOperationId, ContractTypeDescriptor, ContractTypeRef,
    PackageSchemaTypeId, PackageSchemaTypeRecord, PackageSchemaTypeRef, ServiceContract,
};

pub const WEBSOCKET_INGRESS_OPERATION_NAME: &str = "websocket";
pub const WEBSOCKET_INGRESS_EVENT_TYPE: &str = "std.websocket.WebSocketIngressEvent";
pub const WEBSOCKET_CONNECT_RESULT_TYPE: &str = "std.websocket.WebSocketConnectResult";

/// The only WebSocket names admitted into the public ServiceContract builtin vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketContractBuiltin {
    Event,
    Result,
}

/// A canonical nested WebSocket shape. These identifiers are internal shape vocabulary, not
/// additional ServiceContract builtin names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WebSocketShapeId {
    Event,
    ConnectRequest,
    ReceiveEvent,
    Connection,
    Message,
    Result,
    ConnectionPolicy,
    HttpHeader,
    HttpQueryParam,
}

impl WebSocketShapeId {
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Event => WEBSOCKET_INGRESS_EVENT_TYPE,
            Self::ConnectRequest => "std.websocket.WebSocketConnectRequest",
            Self::ReceiveEvent => "std.websocket.WebSocketReceiveEvent",
            Self::Connection => "std.websocket.WebSocketConnection",
            Self::Message => "std.websocket.ConnectionMessage",
            Self::Result => WEBSOCKET_CONNECT_RESULT_TYPE,
            Self::ConnectionPolicy => "std.websocket.WebSocketConnectionPolicy",
            Self::HttpHeader => "std.http.HttpHeader",
            Self::HttpQueryParam => "std.http.HttpQueryParam",
        }
    }
}

/// A type reference inside the canonical WebSocket shape graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketShapeType {
    String,
    Integer,
    /// The exact generic Context selected by the enclosing Event/Result contract pair.
    Context,
    Shape(WebSocketShapeId),
    Array(Box<WebSocketShapeType>),
    Nullable(Box<WebSocketShapeType>),
    StringLiteral(&'static str),
    StringLiteralUnion(Vec<&'static str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketShapeField {
    name: &'static str,
    ty: WebSocketShapeType,
}

impl WebSocketShapeField {
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn ty(&self) -> &WebSocketShapeType {
        &self.ty
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketTaggedVariant {
    canonical_name: &'static str,
    fields: Vec<WebSocketShapeField>,
}

impl WebSocketTaggedVariant {
    pub const fn canonical_name(&self) -> &'static str {
        self.canonical_name
    }

    /// Returns the tag stored in the variant's exact discriminator field.
    pub fn tag(&self, discriminator_field: &str) -> Option<&'static str> {
        let field = self.fields.first()?;
        if field.name != discriminator_field {
            return None;
        }
        match &field.ty {
            WebSocketShapeType::StringLiteral(tag) => Some(*tag),
            _ => None,
        }
    }

    pub fn fields(&self) -> &[WebSocketShapeField] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketShape {
    Record {
        fields: Vec<WebSocketShapeField>,
    },
    TaggedUnion {
        discriminator_field: &'static str,
        variants: Vec<WebSocketTaggedVariant>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebSocketContractBuiltinSpec {
    builtin: WebSocketContractBuiltin,
    name: &'static str,
    context_arity: usize,
    shape: WebSocketShapeId,
}

impl WebSocketContractBuiltinSpec {
    pub const fn builtin(&self) -> WebSocketContractBuiltin {
        self.builtin
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn context_arity(&self) -> usize {
        self.context_arity
    }

    pub const fn shape(&self) -> WebSocketShapeId {
        self.shape
    }
}

/// The dependency-leaf canonical shape owner consumed by contract normalization, admission and
/// downstream Rust materialization consumers.
#[derive(Debug)]
pub struct CanonicalWebSocketShapeSpec {
    contract_builtins: [WebSocketContractBuiltinSpec; 2],
    shapes: BTreeMap<WebSocketShapeId, WebSocketShape>,
}

impl CanonicalWebSocketShapeSpec {
    pub fn contract_builtins(&self) -> &[WebSocketContractBuiltinSpec] {
        &self.contract_builtins
    }

    pub fn contract_builtin(
        &self,
        builtin: WebSocketContractBuiltin,
    ) -> &WebSocketContractBuiltinSpec {
        self.contract_builtins
            .iter()
            .find(|spec| spec.builtin == builtin)
            .expect("canonical WebSocket builtin is present")
    }

    pub fn contract_builtin_named(&self, name: &str) -> Option<&WebSocketContractBuiltinSpec> {
        self.contract_builtins.iter().find(|spec| spec.name == name)
    }

    pub fn shape(&self, shape: WebSocketShapeId) -> &WebSocketShape {
        self.shapes
            .get(&shape)
            .expect("canonical WebSocket shape is present")
    }

    pub fn shapes(
        &self,
    ) -> impl ExactSizeIterator<Item = (WebSocketShapeId, &WebSocketShape)> + '_ {
        self.shapes.iter().map(|(shape, spec)| (*shape, spec))
    }
}

/// Returns the one immutable canonical WebSocket shape graph.
pub fn canonical_websocket_shape_spec() -> &'static CanonicalWebSocketShapeSpec {
    static SPEC: OnceLock<CanonicalWebSocketShapeSpec> = OnceLock::new();
    SPEC.get_or_init(build_canonical_websocket_shape_spec)
}

fn build_canonical_websocket_shape_spec() -> CanonicalWebSocketShapeSpec {
    use WebSocketShapeId as Id;

    let shapes = BTreeMap::from([
        (Id::HttpHeader, http_name_value_shape()),
        (Id::HttpQueryParam, http_name_value_shape()),
        (Id::ConnectRequest, websocket_connect_request_shape()),
        (Id::Connection, websocket_connection_shape()),
        (Id::Message, websocket_message_shape()),
        (Id::ReceiveEvent, websocket_receive_event_shape()),
        (Id::Event, websocket_event_shape()),
        (Id::ConnectionPolicy, websocket_connection_policy_shape()),
        (Id::Result, websocket_result_shape()),
    ]);
    CanonicalWebSocketShapeSpec {
        contract_builtins: [
            WebSocketContractBuiltinSpec {
                builtin: WebSocketContractBuiltin::Event,
                name: WEBSOCKET_INGRESS_EVENT_TYPE,
                context_arity: 1,
                shape: Id::Event,
            },
            WebSocketContractBuiltinSpec {
                builtin: WebSocketContractBuiltin::Result,
                name: WEBSOCKET_CONNECT_RESULT_TYPE,
                context_arity: 1,
                shape: Id::Result,
            },
        ],
        shapes,
    }
}

fn http_name_value_shape() -> WebSocketShape {
    record(vec![
        field("name", WebSocketShapeType::String),
        field("value", WebSocketShapeType::String),
    ])
}

fn websocket_connect_request_shape() -> WebSocketShape {
    use WebSocketShapeId as Id;
    use WebSocketShapeType as Ty;

    record(vec![
        field("connectionId", Ty::String),
        field("url", Ty::String),
        field("query", array(Ty::Shape(Id::HttpQueryParam))),
        field("headers", array(Ty::Shape(Id::HttpHeader))),
        field("cookies", array(Ty::Shape(Id::HttpHeader))),
        field("version", nullable(Ty::String)),
    ])
}

fn websocket_connection_shape() -> WebSocketShape {
    use WebSocketShapeType as Ty;

    record(vec![
        field("id", Ty::String),
        field("businessIdentity", nullable(Ty::String)),
        field("context", Ty::Context),
    ])
}

fn websocket_message_shape() -> WebSocketShape {
    use WebSocketShapeType as Ty;

    tagged_union(
        "tag",
        vec![
            tagged_variant(
                "std.websocket.TextConnectionMessage",
                "tag",
                "text",
                vec![field("text", Ty::String)],
            ),
            tagged_variant(
                "std.websocket.BinaryConnectionMessage",
                "tag",
                "binary",
                vec![field("base64", Ty::String)],
            ),
        ],
    )
}

fn websocket_receive_event_shape() -> WebSocketShape {
    use WebSocketShapeId as Id;
    use WebSocketShapeType as Ty;

    record(vec![
        field("connection", Ty::Shape(Id::Connection)),
        field("message", Ty::Shape(Id::Message)),
    ])
}

fn websocket_event_shape() -> WebSocketShape {
    use WebSocketShapeId as Id;
    use WebSocketShapeType as Ty;

    tagged_union(
        "tag",
        vec![
            tagged_variant(
                "std.websocket.WebSocketIngressConnectEvent",
                "tag",
                "connect",
                vec![field("connectRequest", Ty::Shape(Id::ConnectRequest))],
            ),
            tagged_variant(
                "std.websocket.WebSocketIngressReceiveEvent",
                "tag",
                "receive",
                vec![field("receiveEvent", Ty::Shape(Id::ReceiveEvent))],
            ),
        ],
    )
}

fn websocket_connection_policy_shape() -> WebSocketShape {
    use WebSocketShapeType as Ty;

    record(vec![
        field("maxConnections", Ty::Integer),
        field(
            "overflow",
            Ty::StringLiteralUnion(vec!["close-oldest", "reject-new"]),
        ),
        field("closeCode", nullable(Ty::Integer)),
        field("closeReason", nullable(Ty::String)),
    ])
}

fn websocket_result_shape() -> WebSocketShape {
    use WebSocketShapeId as Id;
    use WebSocketShapeType as Ty;

    tagged_union(
        "tag",
        vec![
            tagged_variant(
                "std.websocket.WebSocketConnectAccept",
                "tag",
                "accept",
                vec![
                    field("context", Ty::Context),
                    field("businessIdentity", nullable(Ty::String)),
                    field(
                        "connectionPolicy",
                        nullable(Ty::Shape(Id::ConnectionPolicy)),
                    ),
                ],
            ),
            tagged_variant(
                "std.websocket.WebSocketConnectReject",
                "tag",
                "reject",
                vec![field("code", Ty::Integer), field("reason", Ty::String)],
            ),
        ],
    )
}

fn field(name: &'static str, ty: WebSocketShapeType) -> WebSocketShapeField {
    WebSocketShapeField { name, ty }
}

fn record(fields: Vec<WebSocketShapeField>) -> WebSocketShape {
    WebSocketShape::Record { fields }
}

fn tagged_union(
    discriminator_field: &'static str,
    variants: Vec<WebSocketTaggedVariant>,
) -> WebSocketShape {
    WebSocketShape::TaggedUnion {
        discriminator_field,
        variants,
    }
}

fn tagged_variant(
    canonical_name: &'static str,
    discriminator_field: &'static str,
    tag: &'static str,
    fields: Vec<WebSocketShapeField>,
) -> WebSocketTaggedVariant {
    let mut tagged_fields = Vec::with_capacity(fields.len() + 1);
    tagged_fields.push(field(
        discriminator_field,
        WebSocketShapeType::StringLiteral(tag),
    ));
    tagged_fields.extend(fields);
    WebSocketTaggedVariant {
        canonical_name,
        fields: tagged_fields,
    }
}

fn array(item: WebSocketShapeType) -> WebSocketShapeType {
    WebSocketShapeType::Array(Box::new(item))
}

fn nullable(inner: WebSocketShapeType) -> WebSocketShapeType {
    WebSocketShapeType::Nullable(Box::new(inner))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketIngressContext {
    Null,
    PackageSchema(PackageSchemaTypeRef),
}

impl WebSocketIngressContext {
    pub fn package_schema_type(&self) -> Option<&PackageSchemaTypeRef> {
        match self {
            Self::Null => None,
            Self::PackageSchema(reference) => Some(reference),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketIngressContractError {
    message: String,
}

impl WebSocketIngressContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WebSocketIngressContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WebSocketIngressContractError {}

/// Validates the single canonical WebSocket ingress ABI and returns its contract-owned Context.
pub fn websocket_ingress_context(
    contract: &ServiceContract,
    operation_id: &ContractOperationId,
    package_schema_records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
) -> Result<WebSocketIngressContext, WebSocketIngressContractError> {
    websocket_ingress_context_with_shape_spec(
        contract,
        operation_id,
        package_schema_records,
        canonical_websocket_shape_spec(),
    )
}

fn websocket_ingress_context_with_shape_spec(
    contract: &ServiceContract,
    operation_id: &ContractOperationId,
    package_schema_records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    shape_spec: &CanonicalWebSocketShapeSpec,
) -> Result<WebSocketIngressContext, WebSocketIngressContractError> {
    let event_builtin = shape_spec.contract_builtin(WebSocketContractBuiltin::Event);
    let result_builtin = shape_spec.contract_builtin(WebSocketContractBuiltin::Result);
    let descriptor = contract.operations.get(operation_id).ok_or_else(|| {
        WebSocketIngressContractError::new(format!(
            "contract has no WebSocket ingress operation {operation_id}"
        ))
    })?;
    if descriptor.stable_key != WEBSOCKET_INGRESS_OPERATION_NAME {
        return Err(WebSocketIngressContractError::new(format!(
            "WebSocket ingress operation must be named {WEBSOCKET_INGRESS_OPERATION_NAME}"
        )));
    }
    let operation = &descriptor.contract;
    let [parameter] = operation.parameters.as_slice() else {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must declare exactly one event parameter",
        ));
    };
    if parameter.name != "event" {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress parameter must be named event",
        ));
    }
    let context = generic_context_argument(&parameter.ty, event_builtin).ok_or_else(|| {
        WebSocketIngressContractError::new(format!(
            "WebSocket ingress event must be {}<Context>",
            event_builtin.name()
        ))
    })?;
    let context = match context {
        ContractTypeRef::Builtin { name, arguments } if name == "null" && arguments.is_empty() => {
            WebSocketIngressContext::Null
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let reference = PackageSchemaTypeRef {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                package_schema_type_id: package_schema_type_id.clone(),
            };
            validate_persistable_context(contract, package_schema_records, &reference)?;
            WebSocketIngressContext::PackageSchema(reference)
        }
        _ => {
            return Err(WebSocketIngressContractError::new(
                "WebSocket ingress Context must be null or a package-owned nominal type",
            ))
        }
    };
    let ContractTypeRef::Nullable { inner } = &operation.return_value.ty else {
        return Err(WebSocketIngressContractError::new(format!(
            "WebSocket ingress return must be {WEBSOCKET_CONNECT_RESULT_TYPE}<Context>?"
        )));
    };
    let return_context = generic_context_argument(inner, result_builtin).ok_or_else(|| {
        WebSocketIngressContractError::new(format!(
            "WebSocket ingress return must be {}<Context>?",
            result_builtin.name()
        ))
    })?;
    if return_context != &generic_context_ref(&context) {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress event and result Context must be identical",
        ));
    }
    if !matches!(operation.errors, BoundaryErrorContract::None) {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must not declare throws",
        ));
    }
    if !matches!(operation.stream, BoundaryStreamContract::Unary) {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must be unary",
        ));
    }
    if operation.may_suspend {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must not suspend",
        ));
    }
    if !matches!(
        operation.cancellation,
        BoundaryCancellationContract::NotCancellable
    ) || !matches!(operation.callbacks, BoundaryCallbackContract::None)
    {
        return Err(WebSocketIngressContractError::new(
            "WebSocket ingress operation must not declare cancellation or callbacks",
        ));
    }
    Ok(context)
}

fn validate_persistable_context(
    contract: &ServiceContract,
    package_schema_records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    context_type: &PackageSchemaTypeRef,
) -> Result<(), WebSocketIngressContractError> {
    let mut visiting = BTreeSet::new();
    let mut complete = BTreeSet::new();
    visit_persistable_context_type(
        contract,
        package_schema_records,
        context_type,
        "WebSocket ingress Context",
        &mut visiting,
        &mut complete,
    )
}

fn visit_persistable_context_type(
    contract: &ServiceContract,
    package_schema_records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    package_schema_type: &PackageSchemaTypeRef,
    path: &str,
    visiting: &mut BTreeSet<PackageSchemaTypeId>,
    complete: &mut BTreeSet<PackageSchemaTypeId>,
) -> Result<(), WebSocketIngressContractError> {
    let type_id = &package_schema_type.package_schema_type_id;
    if complete.contains(type_id) {
        return Ok(());
    }
    let required = contract
        .package_type_requirements
        .iter()
        .any(|requirement| {
            requirement.package_id == package_schema_type.package_id
                && requirement.required_type_ids.binary_search(type_id).is_ok()
        });
    if !required {
        return Err(WebSocketIngressContractError::new(format!(
            "{path} references package schema type {type_id} outside ServiceContract requirements"
        )));
    }
    let schema_type = package_schema_records.get(type_id).ok_or_else(|| {
        WebSocketIngressContractError::new(format!(
            "{path} references missing PackageSchemaTypeId {type_id}"
        ))
    })?;
    if schema_type.package_id != package_schema_type.package_id
        || schema_type.stable_schema_key != package_schema_type.stable_schema_key
        || &schema_type.package_schema_type_id != type_id
    {
        return Err(WebSocketIngressContractError::new(format!(
            "{path} package schema owner, key, or identity does not match its record"
        )));
    }
    if !visiting.insert(type_id.clone()) {
        return Err(WebSocketIngressContractError::new(format!(
            "{path} contains a package schema cycle at {}",
            schema_type.stable_schema_key
        )));
    }

    let schema_path = format!("{path}.{}", schema_type.stable_schema_key);
    let result = match &schema_type.canonical_descriptor.descriptor {
        ContractTypeDescriptor::Record { fields } => fields.iter().try_for_each(|(name, ty)| {
            validate_persistable_context_ref(
                contract,
                package_schema_records,
                ty,
                &format!("{schema_path}.{name}"),
                visiting,
                complete,
            )
        }),
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants.iter().enumerate().try_for_each(|(index, ty)| {
                validate_persistable_context_ref(
                    contract,
                    package_schema_records,
                    ty,
                    &format!("{schema_path}.variants[{index}]"),
                    visiting,
                    complete,
                )
            })
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            branches.iter().try_for_each(|branch| {
                validate_persistable_context_ref(
                    contract,
                    package_schema_records,
                    &branch.branch_type,
                    &format!("{schema_path}.branches[{}]", branch.tag),
                    visiting,
                    complete,
                )
            })
        }
        ContractTypeDescriptor::Representation { target } => validate_persistable_context_ref(
            contract,
            package_schema_records,
            target,
            &format!("{schema_path}.target"),
            visiting,
            complete,
        ),
        ContractTypeDescriptor::Enumeration { .. } => Ok(()),
        ContractTypeDescriptor::Alias { .. } => Err(WebSocketIngressContractError::new(format!(
            "{schema_path} is a transparent alias, not an exact persistable nominal type"
        ))),
        ContractTypeDescriptor::CallbackInterface { .. } => {
            Err(WebSocketIngressContractError::new(format!(
                "{schema_path} is a CallbackInterface and cannot be persisted as WebSocket Context"
            )))
        }
    };

    visiting.remove(type_id);
    if result.is_ok() {
        complete.insert(type_id.clone());
    }
    result
}

fn validate_persistable_context_ref(
    contract: &ServiceContract,
    package_schema_records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    ty: &ContractTypeRef,
    path: &str,
    visiting: &mut BTreeSet<PackageSchemaTypeId>,
    complete: &mut BTreeSet<PackageSchemaTypeId>,
) -> Result<(), WebSocketIngressContractError> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            if name == "void" {
                return Err(WebSocketIngressContractError::new(format!(
                    "{path} uses non-persistable builtin void"
                )));
            }
            arguments.iter().enumerate().try_for_each(|(index, ty)| {
                validate_persistable_context_ref(
                    contract,
                    package_schema_records,
                    ty,
                    &format!("{path}.arguments[{index}]"),
                    visiting,
                    complete,
                )
            })
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => visit_persistable_context_type(
            contract,
            package_schema_records,
            &PackageSchemaTypeRef {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                package_schema_type_id: package_schema_type_id.clone(),
            },
            path,
            visiting,
            complete,
        ),
        ContractTypeRef::TypeParam { name } => Err(WebSocketIngressContractError::new(format!(
            "{path} contains unresolved type parameter {name}"
        ))),
        ContractTypeRef::Record { fields } => fields.iter().try_for_each(|(name, ty)| {
            validate_persistable_context_ref(
                contract,
                package_schema_records,
                ty,
                &format!("{path}.{name}"),
                visiting,
                complete,
            )
        }),
        ContractTypeRef::StructuralUnion { variants } => {
            variants.iter().enumerate().try_for_each(|(index, ty)| {
                validate_persistable_context_ref(
                    contract,
                    package_schema_records,
                    ty,
                    &format!("{path}.variants[{index}]"),
                    visiting,
                    complete,
                )
            })
        }
        ContractTypeRef::Nullable { inner } => validate_persistable_context_ref(
            contract,
            package_schema_records,
            inner,
            &format!("{path}.inner"),
            visiting,
            complete,
        ),
        ContractTypeRef::Literal { .. } => Ok(()),
    }
}

fn generic_context_argument<'a>(
    ty: &'a ContractTypeRef,
    builtin: &WebSocketContractBuiltinSpec,
) -> Option<&'a ContractTypeRef> {
    let ContractTypeRef::Builtin { name, arguments } = ty else {
        return None;
    };
    if name != builtin.name() || arguments.len() != builtin.context_arity() {
        return None;
    }
    arguments.first()
}

fn generic_context_ref(context: &WebSocketIngressContext) -> ContractTypeRef {
    match context {
        WebSocketIngressContext::Null => ContractTypeRef::builtin("null"),
        WebSocketIngressContext::PackageSchema(reference) => ContractTypeRef::package_schema(
            reference.package_id.clone(),
            reference.stable_schema_key.clone(),
            reference.package_schema_type_id.clone(),
        ),
    }
}

#[cfg(test)]
mod tests;
