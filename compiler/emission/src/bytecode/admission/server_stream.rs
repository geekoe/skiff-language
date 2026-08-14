use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{gateway_entry_identity, normalize_gateway_external_schema};
use skiff_artifact_model::{
    http_boundary::{HTTP_BOUNDARY_PACKAGE_ID, HTTP_REQUEST_TYPE, HTTP_RESPONSE_STREAM_EVENT_TYPE},
    BuiltinReceiverMethod, BuiltinReceiverRoot, CallTargetIr, DeploymentGatewayEntry, ExprIr,
    GatewayAdapterKind, GatewayAdapterSource, GatewayDispatchMode, GatewayExternalErrorProjection,
    GatewayExternalSchema, GatewayProtocolSurface, LiteralIr, PackageCallableId, PackageRefIr,
    TypeRefIr,
};
use skiff_compiler_lowering::mir::{MirFunction, MirStmtKind, MirUnit};

const HTTP_HEADER_TYPE: &str = "std.http.HttpHeader";
const HTTP_QUERY_PARAM_TYPE: &str = "std.http.HttpQueryParam";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ServerStreamEmitFact {
    statement_index: u32,
    value_expression: u32,
}

impl ServerStreamEmitFact {
    pub fn new(statement_index: u32, value_expression: u32) -> Self {
        Self {
            statement_index,
            value_expression,
        }
    }
}

/// Untrusted transport of one exact output from the canonical HTTP gateway
/// projector. Admission rechecks the typed entry, handler, nominal ABI and
/// all-and-only MIR Emit edges before granting any server-stream capability.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerStreamGatewayAuthority {
    entry: DeploymentGatewayEntry,
    stream_item_type: TypeRefIr,
    emit_facts: Vec<ServerStreamEmitFact>,
}

impl ServerStreamGatewayAuthority {
    pub fn new(
        entry: DeploymentGatewayEntry,
        stream_item_type: TypeRefIr,
        emit_facts: Vec<ServerStreamEmitFact>,
    ) -> Self {
        Self {
            entry,
            stream_item_type,
            emit_facts,
        }
    }

    pub fn handler(&self) -> Option<&PackageCallableId> {
        self.entry.handler.as_ref()
    }
}

pub(super) fn validate_authority_coverage(
    units: &[MirUnit],
    transported: &[ServerStreamGatewayAuthority],
) -> Result<(), String> {
    let mut handlers = BTreeMap::<&PackageCallableId, usize>::new();
    for unit in units {
        for function in &unit.functions {
            *handlers.entry(&function.effect_summary_ref).or_default() += 1;
        }
    }
    let mut transported_handlers = BTreeSet::new();
    for authority in transported {
        let handler = authority
            .handler()
            .ok_or_else(|| "gateway authority lacks an exact handler".to_string())?;
        if !transported_handlers.insert(handler) {
            return Err(format!(
                "multiple gateway authorities name handler {handler}"
            ));
        }
        if handlers.get(handler) != Some(&1) {
            return Err(format!(
                "gateway handler {handler} does not name exactly one MIR function"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ServerStreamValueRole {
    Request,
    RequestField(String),
    RequestBodyUtf8,
    ResponseBranch(String),
    ResponseField { tag: String, field: String },
}

#[derive(Debug, Clone)]
struct ServerStreamValueAuthority {
    role: ServerStreamValueRole,
    ty: TypeRefIr,
}

#[derive(Debug, Default)]
pub(super) struct ServerStreamAdmissions {
    result_type: Option<TypeRefIr>,
    slots: BTreeMap<u32, TypeRefIr>,
    expressions: BTreeMap<u32, Vec<ServerStreamValueAuthority>>,
    construct_types: BTreeMap<u32, TypeRefIr>,
    receiver_calls: BTreeSet<u32>,
    emit_statements: BTreeMap<u32, u32>,
}

impl ServerStreamAdmissions {
    pub(super) fn analyze(
        unit: &MirUnit,
        function: &MirFunction,
        transported: &[ServerStreamGatewayAuthority],
    ) -> Result<Self, String> {
        let mut matches = transported
            .iter()
            .filter(|authority| authority.handler() == Some(&function.effect_summary_ref));
        let Some(authority) = matches.next() else {
            return Ok(Self::default());
        };
        if matches.next().is_some() {
            return Err("multiple gateway authorities name the same callable".to_string());
        }
        validate_gateway_entry(&authority.entry)?;

        let stream = function
            .stream_result
            .as_ref()
            .ok_or_else(|| "gateway authority names a non-stream function".to_string())?;
        if stream.item_type != authority.stream_item_type
            || function.return_type
                != (TypeRefIr::Builtin {
                    name: "Stream".to_string(),
                    args: vec![authority.stream_item_type.clone()],
                })
        {
            return Err("gateway stream item differs from producer-owned MIR facts".to_string());
        }
        let abi = exact_std_symbol_abi(
            unit,
            &authority.stream_item_type,
            HTTP_RESPONSE_STREAM_EVENT_TYPE,
        )?;

        let [adapter_arg] = authority.entry.adapter_plan.args.as_slice() else {
            return Err("server-stream gateway must have one exact adapter argument".to_string());
        };
        let [parameter] = function.params.as_slice() else {
            return Err("server-stream handler must have one exact parameter".to_string());
        };
        if adapter_arg.source != GatewayAdapterSource::HttpRequest
            || adapter_arg.param != parameter.name
        {
            return Err("gateway http.request argument differs from the MIR parameter".to_string());
        }
        let request_abi = exact_std_symbol_abi(unit, &parameter.ty, HTTP_REQUEST_TYPE)?;
        if request_abi != abi {
            return Err("gateway request and stream item use different std ABIs".to_string());
        }
        let request_fields = exact_http_request_fields(unit, &abi)?;

        let actual_emits = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.kind {
                MirStmtKind::Emit { operation, value } if operation.is_empty() => Some(
                    ServerStreamEmitFact::new(statement.statement_index, value.expression),
                ),
                MirStmtKind::Emit { .. } => None,
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let transported_emits = authority
            .emit_facts
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if actual_emits.is_empty()
            || transported_emits.len() != authority.emit_facts.len()
            || actual_emits != transported_emits
        {
            return Err("gateway Emit facts are missing, duplicated, or extra".to_string());
        }

        let mut admissions = Self {
            result_type: Some(authority.stream_item_type.clone()),
            ..Self::default()
        };
        admissions
            .slots
            .insert(parameter.slot, parameter.ty.clone());
        for fact in actual_emits {
            admissions
                .emit_statements
                .insert(fact.statement_index, fact.value_expression);
            admissions.admit_response_branch(
                unit,
                function,
                fact.value_expression,
                &authority.stream_item_type,
                &abi,
            )?;
        }

        for expression in &function.expressions {
            if matches!(expression.expression, ExprIr::LoadSlot { slot } if slot == parameter.slot)
                && expression.ty == parameter.ty
            {
                admissions.insert_expression(
                    expression.index,
                    ServerStreamValueRole::Request,
                    expression.ty.clone(),
                );
            }
        }
        for expression in &function.expressions {
            let ExprIr::Field { object, field } = &expression.expression else {
                continue;
            };
            if !admissions.has_role(object.expression, &ServerStreamValueRole::Request) {
                continue;
            }
            let Some(expected) = request_fields.get(field) else {
                return Err(format!(
                    "HttpRequest field `{field}` is absent from exact records"
                ));
            };
            if expected != &expression.ty {
                return Err(format!("HttpRequest field `{field}` type drifted"));
            }
            admissions.insert_expression(
                expression.index,
                ServerStreamValueRole::RequestField(field.clone()),
                expression.ty.clone(),
            );
        }
        admissions.admit_request_body_utf8(function)?;
        Ok(admissions)
    }

    pub(super) fn admits_result(&self, ty: &TypeRefIr) -> bool {
        self.result_type.as_ref() == Some(ty)
    }

    pub(super) fn admits_slot(&self, slot: u32, ty: &TypeRefIr) -> bool {
        self.slots.get(&slot) == Some(ty)
    }

    pub(super) fn admits_expression(&self, expression: u32, ty: &TypeRefIr) -> bool {
        self.expressions
            .get(&expression)
            .is_some_and(|authorities| authorities.iter().any(|authority| &authority.ty == ty))
    }

    pub(super) fn admits_construct(&self, expression: u32, ty: &TypeRefIr) -> bool {
        self.construct_types.get(&expression) == Some(ty)
    }

    pub(super) fn admits_receiver_call(&self, expression: u32) -> bool {
        self.receiver_calls.contains(&expression)
    }

    pub(super) fn admits_tag_literal(&self, expression: u32, value: &str) -> bool {
        self.expressions
            .get(&expression)
            .is_some_and(|authorities| {
                authorities.iter().any(|authority| {
                    matches!(
                        &authority.role,
                        ServerStreamValueRole::ResponseField { tag, field }
                            if field == "tag" && tag == value
                    )
                })
            })
    }

    pub(super) fn admits_emit(&self, statement: u32, expression: u32, ty: &TypeRefIr) -> bool {
        self.emit_statements.get(&statement) == Some(&expression)
            && self.admits_expression(expression, ty)
    }

    pub(super) fn admits_null_return(
        &self,
        function: &MirFunction,
        value: skiff_artifact_model::ExprRefIr,
    ) -> bool {
        self.result_type.is_some()
            && function.expression(value).is_ok_and(|expression| {
                matches!(
                    (&expression.expression, &expression.ty),
                    (
                        ExprIr::Literal {
                            value: LiteralIr::Null
                        },
                        TypeRefIr::Literal {
                            value: LiteralIr::Null
                        }
                    )
                )
            })
    }

    fn admit_response_branch(
        &mut self,
        unit: &MirUnit,
        function: &MirFunction,
        expression_index: u32,
        item_type: &TypeRefIr,
        abi: &str,
    ) -> Result<(), String> {
        let expression = function
            .expression(skiff_artifact_model::ExprRefIr {
                expression: expression_index,
            })
            .map_err(|error| format!("Emit value expression is absent: {error}"))?;
        let ExprIr::Construct { type_ref, fields } = &expression.expression else {
            return Err("server-stream Emit value is not an exact union construction".to_string());
        };
        if type_ref != item_type {
            return Err("server-stream construction nominal differs from its item".to_string());
        }
        let TypeRefIr::Record {
            fields: branch_fields,
        } = &expression.ty
        else {
            return Err("server-stream construction lacks an exact branch record".to_string());
        };
        if fields.keys().collect::<BTreeSet<_>>() != branch_fields.keys().collect::<BTreeSet<_>>() {
            return Err("server-stream construction field facts disagree".to_string());
        }
        let tag = branch_tag(function, fields)?;
        validate_response_branch(unit, branch_fields, &tag, abi)?;
        self.construct_types
            .insert(expression_index, item_type.clone());
        self.insert_expression(
            expression_index,
            ServerStreamValueRole::ResponseBranch(tag.clone()),
            expression.ty.clone(),
        );
        for (field, value) in fields {
            let value_expression = function
                .expression(*value)
                .map_err(|error| format!("response field `{field}` is absent: {error}"))?;
            let expected = &branch_fields[field];
            if &value_expression.ty != expected {
                return Err(format!("response field `{field}` type drifted"));
            }
            self.insert_expression(
                value.expression,
                ServerStreamValueRole::ResponseField {
                    tag: tag.clone(),
                    field: field.clone(),
                },
                expected.clone(),
            );
        }
        Ok(())
    }

    fn admit_request_body_utf8(&mut self, function: &MirFunction) -> Result<(), String> {
        for expression in &function.expressions {
            let ExprIr::Call { call } = &expression.expression else {
                continue;
            };
            let CallTargetIr::ReceiverBuiltin { op } = &call.target else {
                continue;
            };
            let [argument] = call.args.as_slice() else {
                continue;
            };
            if op.receiver != BuiltinReceiverRoot::Bytes
                || op.method != BuiltinReceiverMethod::ToUtf8String
                || op.signature_version != 1
                || op.canonical_key != "receiver:bytes.toUtf8String@1"
                || !call.inout_args.is_empty()
                || !call.type_args.is_empty()
                || call.concrete_receiver.is_some()
                || !call.metadata.is_empty()
                || expression.ty != TypeRefIr::builtin("string")
                || !self.has_role(
                    argument.expression,
                    &ServerStreamValueRole::RequestField("body".to_string()),
                )
            {
                continue;
            }
            self.receiver_calls.insert(expression.index);
            self.insert_expression(
                expression.index,
                ServerStreamValueRole::RequestBodyUtf8,
                expression.ty.clone(),
            );
        }
        Ok(())
    }

    fn insert_expression(&mut self, expression: u32, role: ServerStreamValueRole, ty: TypeRefIr) {
        self.expressions
            .entry(expression)
            .or_default()
            .push(ServerStreamValueAuthority { role, ty });
    }

    fn has_role(&self, expression: u32, role: &ServerStreamValueRole) -> bool {
        self.expressions
            .get(&expression)
            .is_some_and(|authorities| authorities.iter().any(|authority| &authority.role == role))
    }
}

fn validate_gateway_entry(entry: &DeploymentGatewayEntry) -> Result<(), String> {
    if gateway_entry_identity(&entry.protocol_surface).map_err(|error| error.to_string())?
        != entry.gateway_entry_identity
    {
        return Err("gateway entry identity differs from its typed surface".to_string());
    }
    let GatewayProtocolSurface::Http(http) = &entry.protocol_surface.protocol else {
        return Err("server-stream authority is not an HTTP gateway".to_string());
    };
    if http.adapter_kind != GatewayAdapterKind::RawHttp
        || http.dispatch_mode != GatewayDispatchMode::ServerStream
        || http.external_sources.as_slice() != [GatewayAdapterSource::HttpRequest]
        || http.request_body_schema.is_some()
        || http.response_schema.is_some()
        || http.stream_item_schema.as_ref() != Some(&canonical_response_stream_schema()?)
        || entry.protocol_surface.external_error_projection
            != GatewayExternalErrorProjection::FIXED_V1
        || entry.adapter_plan.kind != GatewayAdapterKind::RawHttp
        || entry.pre.is_some()
        || entry.guard.is_some()
        || entry.close_handler.is_some()
        || entry.close_adapter_plan.is_some()
    {
        return Err("gateway entry is not the exact rawHttp server-stream surface".to_string());
    }
    Ok(())
}

fn canonical_response_stream_schema() -> Result<GatewayExternalSchema, String> {
    let header = GatewayExternalSchema::Record {
        fields: BTreeMap::from([
            ("name".to_string(), GatewayExternalSchema::String),
            ("value".to_string(), GatewayExternalSchema::String),
        ]),
        required: vec!["name".to_string(), "value".to_string()],
    };
    let record = |fields: BTreeMap<String, GatewayExternalSchema>| {
        let required = fields.keys().cloned().collect();
        GatewayExternalSchema::Record { fields, required }
    };
    normalize_gateway_external_schema(GatewayExternalSchema::ClosedUnion {
        branches: vec![
            record(BTreeMap::from([
                (
                    "tag".to_string(),
                    GatewayExternalSchema::StringLiteral {
                        value: "start".to_string(),
                    },
                ),
                ("status".to_string(), GatewayExternalSchema::Integer),
                (
                    "headers".to_string(),
                    GatewayExternalSchema::Array {
                        items: Box::new(header),
                    },
                ),
            ])),
            record(BTreeMap::from([
                (
                    "tag".to_string(),
                    GatewayExternalSchema::StringLiteral {
                        value: "chunk".to_string(),
                    },
                ),
                ("value".to_string(), GatewayExternalSchema::Bytes),
            ])),
            record(BTreeMap::from([(
                "tag".to_string(),
                GatewayExternalSchema::StringLiteral {
                    value: "end".to_string(),
                },
            )])),
        ],
    })
    .map_err(|error| error.to_string())
}

fn exact_std_symbol_abi(unit: &MirUnit, ty: &TypeRefIr, path: &str) -> Result<String, String> {
    let TypeRefIr::PackageSymbol { symbol } = ty else {
        return Err(format!("{path} uses a non-package type form"));
    };
    let PackageRefIr::PackageId { package_id } = &symbol.package else {
        return Err(format!("{path} retains a dependency alias"));
    };
    let abi = symbol
        .abi_expectation
        .as_deref()
        .filter(|abi| !abi.trim().is_empty())
        .ok_or_else(|| format!("{path} lacks a nonempty ABI"))?;
    if package_id != HTTP_BOUNDARY_PACKAGE_ID || symbol.symbol_path != path {
        return Err(format!("{path} owner/path drifted"));
    }
    let matching = unit
        .external_refs
        .package_symbols
        .iter()
        .filter(|candidate| {
            matches!(
                &candidate.package,
                PackageRefIr::PackageId { package_id: candidate_owner }
                    if candidate_owner == package_id
            ) && candidate.symbol_path == path
                && candidate
                    .abi_expectation
                    .as_deref()
                    .is_some_and(|candidate_abi| {
                        !candidate_abi.trim().is_empty() && candidate_abi == abi
                    })
        })
        .collect::<Vec<_>>();
    if matching.as_slice() != [symbol] {
        return Err(format!("{path} lacks one exact external-ref authority"));
    }
    Ok(abi.to_string())
}

fn exact_http_request_fields(
    unit: &MirUnit,
    abi: &str,
) -> Result<BTreeMap<String, TypeRefIr>, String> {
    let fields = exact_record(unit, HTTP_REQUEST_TYPE)?;
    let header = exact_named_record(unit, HTTP_HEADER_TYPE, abi)?;
    let query = exact_named_record(unit, HTTP_QUERY_PARAM_TYPE, abi)?;
    let string = TypeRefIr::builtin("string");
    let expected = BTreeMap::from([
        ("method".to_string(), string.clone()),
        ("url".to_string(), string.clone()),
        ("path".to_string(), string),
        (
            "query".to_string(),
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![query],
            },
        ),
        (
            "headers".to_string(),
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![header],
            },
        ),
        ("body".to_string(), TypeRefIr::builtin("bytes")),
    ]);
    if fields != &expected {
        return Err("HttpRequest package record differs from canonical fields".to_string());
    }
    Ok(expected)
}

fn exact_named_record(unit: &MirUnit, path: &str, abi: &str) -> Result<TypeRefIr, String> {
    let ty = TypeRefIr::PackageSymbol {
        symbol: skiff_artifact_model::PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: HTTP_BOUNDARY_PACKAGE_ID.to_string(),
            },
            symbol_path: path.to_string(),
            abi_expectation: Some(abi.to_string()),
        },
    };
    exact_std_symbol_abi(unit, &ty, path)?;
    let fields = exact_record(unit, path)?;
    if fields
        != &BTreeMap::from([
            ("name".to_string(), TypeRefIr::builtin("string")),
            ("value".to_string(), TypeRefIr::builtin("string")),
        ])
    {
        return Err(format!(
            "{path} package record differs from canonical fields"
        ));
    }
    Ok(ty)
}

fn exact_record<'a>(
    unit: &'a MirUnit,
    path: &str,
) -> Result<&'a BTreeMap<String, TypeRefIr>, String> {
    unit.package_type_records
        .get(&(HTTP_BOUNDARY_PACKAGE_ID.to_string(), path.to_string()))
        .ok_or_else(|| format!("{path} package record is missing"))
}

fn branch_tag(
    function: &MirFunction,
    fields: &BTreeMap<String, skiff_artifact_model::ExprRefIr>,
) -> Result<String, String> {
    let tag = fields
        .get("tag")
        .ok_or_else(|| "response branch lacks tag".to_string())?;
    let expression = function
        .expression(*tag)
        .map_err(|error| format!("response tag expression is absent: {error}"))?;
    let ExprIr::Literal {
        value: LiteralIr::String { value },
    } = &expression.expression
    else {
        return Err("response tag is not an exact string literal".to_string());
    };
    Ok(value.clone())
}

fn validate_response_branch(
    unit: &MirUnit,
    fields: &BTreeMap<String, TypeRefIr>,
    tag: &str,
    abi: &str,
) -> Result<(), String> {
    let tag_type = TypeRefIr::Literal {
        value: LiteralIr::String {
            value: tag.to_string(),
        },
    };
    let expected = match tag {
        "start" => BTreeMap::from([
            ("tag".to_string(), tag_type),
            ("status".to_string(), TypeRefIr::builtin("integer")),
            (
                "headers".to_string(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![exact_named_record(unit, HTTP_HEADER_TYPE, abi)?],
                },
            ),
        ]),
        "chunk" => BTreeMap::from([
            ("tag".to_string(), tag_type),
            ("value".to_string(), TypeRefIr::builtin("bytes")),
        ]),
        "end" => BTreeMap::from([("tag".to_string(), tag_type)]),
        _ => return Err(format!("unsupported response stream tag `{tag}`")),
    };
    if fields != &expected {
        return Err(format!("response stream `{tag}` branch shape drifted"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use skiff_artifact_identity::{
        gateway_entry_identity, normalize_gateway_entry_protocol_surface,
    };
    use skiff_artifact_model::{
        DeploymentGatewayEntry, ExprIr, GatewayAdapterArg, GatewayAdapterKind, GatewayAdapterPlan,
        GatewayAdapterSource, GatewayDispatchMode, GatewayEntryProtocolSurface,
        GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
        GatewayProtocolSurface, NominalTypeRefBaseIr, PackageCallableId, PackageRefIr, TypeRefIr,
    };
    use skiff_compiler_lowering::mir::{
        source_program::{lower_single_source_program, SingleSourceProgram},
        MirStmtKind, MirUnit,
    };

    use super::{
        canonical_response_stream_schema, ServerStreamEmitFact, ServerStreamGatewayAuthority,
        HTTP_BOUNDARY_PACKAGE_ID, HTTP_RESPONSE_STREAM_EVENT_TYPE,
    };

    const SOURCE: &str = r#"
import std

function consume(
  request: std.http.HttpRequest
) -> Stream<std.http.HttpResponseStreamEvent> {
  final outbound = std.http.HttpClientRequest {
    method: request.method,
    url: request.body.toUtf8String(),
    headers: request.headers,
    body: null,
    timeoutMs: null,
  }
  final response = std.http.stream(outbound)
  emit({ tag: "start", status: 207, headers: [] })
  for chunk in response.body {
    emit({ tag: "chunk", value: chunk })
  }
  emit({ tag: "end" })
  return null
}
"#;

    #[test]
    fn exact_projected_gateway_authority_admits_real_server_stream_source() {
        let (mut units, authority) = fixture();
        let mut retained_alias = units[0]
            .external_refs
            .package_symbols
            .iter()
            .find(|symbol| symbol.symbol_path == HTTP_RESPONSE_STREAM_EVENT_TYPE)
            .expect("canonical event row exists")
            .clone();
        retained_alias.package = PackageRefIr::Dependency {
            dependency_ref: "std".to_string(),
        };
        units[0].external_refs.package_symbols.push(retained_alias);
        super::super::admit_phase_1_bytecode_mir_with_server_stream_authorities(
            &units,
            &[authority],
        )
        .expect("exact canonical gateway authority admits the real source carrier");
    }

    #[test]
    fn static_collection_intrinsics_remain_fail_closed_in_exact_server_streams() {
        let array_source =
            SOURCE.replace("headers: []", "headers: Array.empty<std.http.HttpHeader>()");
        let (array_units, array_authority) = fixture_for_source(&array_source);
        assert!(
            super::super::admit_phase_1_bytecode_mir_with_server_stream_authorities(
                &array_units,
                &[array_authority]
            )
            .is_err(),
            "exact response role must not mint Array.empty intrinsic authority"
        );

        let map_source = SOURCE.replace(
            "  final outbound =",
            "  Map.empty<string, string>()\n  final outbound =",
        );
        let (map_units, map_authority) = fixture_for_source(&map_source);
        assert!(
            super::super::admit_phase_1_bytecode_mir_with_server_stream_authorities(
                &map_units,
                &[map_authority]
            )
            .is_err(),
            "exact producer context must not mint Map.empty intrinsic authority"
        );
    }

    #[test]
    fn server_stream_authority_rechecks_handler_identity_protocol_and_abi() {
        let (units, authority) = fixture();
        let mut cases = Vec::new();

        let mut wrong_handler = authority.clone();
        wrong_handler.entry.handler = Some(PackageCallableId::new("forged:handler"));
        cases.push(wrong_handler);

        let mut wrong_identity = authority.clone();
        let alternate_surface = typed_http_protocol_surface();
        wrong_identity.entry.gateway_entry_identity =
            gateway_entry_identity(&alternate_surface).expect("alternate identity is valid");
        cases.push(wrong_identity);

        let mut wrong_protocol = authority.clone();
        wrong_protocol.entry.protocol_surface = alternate_surface;
        wrong_protocol.entry.gateway_entry_identity =
            gateway_entry_identity(&wrong_protocol.entry.protocol_surface)
                .expect("typed protocol has a canonical identity");
        wrong_protocol.entry.adapter_plan = GatewayAdapterPlan {
            kind: GatewayAdapterKind::TypedJson,
            args: vec![GatewayAdapterArg {
                param: "request".to_string(),
                source: GatewayAdapterSource::HttpBody,
            }],
        };
        cases.push(wrong_protocol);

        let mut wrong_abi = authority;
        let TypeRefIr::PackageSymbol { symbol } = &mut wrong_abi.stream_item_type else {
            unreachable!()
        };
        symbol.abi_expectation = Some("sha256:drift".to_string());
        cases.push(wrong_abi);

        for rejected in cases {
            assert!(
                super::super::admit_phase_1_bytecode_mir_with_server_stream_authorities(
                    &units,
                    &[rejected]
                )
                .is_err()
            );
        }
    }

    #[test]
    fn server_stream_authority_requires_all_and_only_emit_facts() {
        let (units, authority) = fixture();
        assert!(
            super::super::admit_phase_1_bytecode_mir_with_server_stream_authorities(&units, &[])
                .is_err(),
            "ordinary Stream/type shape cannot mint gateway authority"
        );

        let mut missing = authority.clone();
        missing.emit_facts.pop();
        assert!(
            super::super::admit_phase_1_bytecode_mir_with_server_stream_authorities(
                &units,
                &[missing]
            )
            .is_err()
        );

        let mut extra = authority;
        extra
            .emit_facts
            .push(ServerStreamEmitFact::new(u32::MAX, u32::MAX));
        assert!(
            super::super::admit_phase_1_bytecode_mir_with_server_stream_authorities(
                &units,
                &[extra]
            )
            .is_err()
        );
    }

    fn fixture() -> (Vec<MirUnit>, ServerStreamGatewayAuthority) {
        fixture_for_source(SOURCE)
    }

    fn fixture_for_source(source: &str) -> (Vec<MirUnit>, ServerStreamGatewayAuthority) {
        let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lowered = lower_single_source_program(SingleSourceProgram {
            platform_root: &platform_root,
            package_id: "example.com/server-stream-authority",
            module_path: "main",
            relative_path: "main.skiff",
            source,
        })
        .expect("real server-stream source lowers");
        let mut units = super::super::package_type_authority::normalize_package_type_authorities(
            lowered.mir_units(),
        )
        .expect("real source package authority normalizes");
        stamp_exact_std_abi(&mut units);
        let function = &units[0].functions[0];
        let stream_item_type = function
            .stream_result
            .as_ref()
            .expect("fixture is a stream producer")
            .item_type
            .clone();
        let emit_facts = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.kind {
                MirStmtKind::Emit { value, .. } => Some(ServerStreamEmitFact::new(
                    statement.statement_index,
                    value.expression,
                )),
                _ => None,
            })
            .collect();
        let protocol_surface =
            normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
                protocol: GatewayProtocolSurface::Http(exact_http_surface()),
                external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
            })
            .expect("fixture protocol surface is canonical");
        let entry = DeploymentGatewayEntry {
            gateway_entry_identity: gateway_entry_identity(&protocol_surface)
                .expect("fixture gateway identity derives"),
            protocol_surface,
            handler: Some(function.effect_summary_ref.clone()),
            pre: None,
            guard: None,
            adapter_plan: GatewayAdapterPlan {
                kind: GatewayAdapterKind::RawHttp,
                args: vec![GatewayAdapterArg {
                    param: "request".to_string(),
                    source: GatewayAdapterSource::HttpRequest,
                }],
            },
            close_handler: None,
            close_adapter_plan: None,
        };
        (
            units,
            ServerStreamGatewayAuthority::new(entry, stream_item_type, emit_facts),
        )
    }

    fn exact_http_surface() -> GatewayHttpProtocolSurface {
        GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::RawHttp,
            dispatch_mode: GatewayDispatchMode::ServerStream,
            external_sources: vec![GatewayAdapterSource::HttpRequest],
            request_body_schema: None,
            response_schema: None,
            stream_item_schema: Some(
                canonical_response_stream_schema().expect("canonical response schema derives"),
            ),
        }
    }

    fn typed_http_protocol_surface() -> GatewayEntryProtocolSurface {
        normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::TypedJson,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::HttpBody],
                request_body_schema: Some(GatewayExternalSchema::String),
                response_schema: Some(GatewayExternalSchema::String),
                stream_item_schema: None,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        })
        .expect("typed protocol remains structurally canonical")
    }

    fn stamp_exact_std_abi(units: &mut [MirUnit]) {
        const ABI: &str = "skiff-package-local-abi-v7:sha256:c5-test-authority";
        for unit in units {
            for symbol in &mut unit.external_refs.package_symbols {
                stamp_symbol(symbol, ABI);
            }
            for fields in unit.package_type_records.values_mut() {
                for ty in fields.values_mut() {
                    stamp_type(ty, ABI);
                }
            }
            for function in &mut unit.functions {
                for parameter in &mut function.params {
                    stamp_type(&mut parameter.ty, ABI);
                }
                stamp_type(&mut function.return_type, ABI);
                for slot in &mut function.slots {
                    if let Some(ty) = &mut slot.ty {
                        stamp_type(ty, ABI);
                    }
                }
                for expression in &mut function.expressions {
                    stamp_type(&mut expression.ty, ABI);
                    if let Some(stream) = &mut expression.stream_result {
                        stamp_type(&mut stream.item_type, ABI);
                    }
                    match &mut expression.expression {
                        ExprIr::Construct { type_ref, .. }
                        | ExprIr::RepresentationWrap { type_ref, .. } => {
                            stamp_type(type_ref, ABI);
                        }
                        ExprIr::Call { call } => {
                            for ty in call.type_args.values_mut() {
                                stamp_type(ty, ABI);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(stream) = &mut function.stream_result {
                    stamp_type(&mut stream.item_type, ABI);
                }
            }
        }
    }

    fn stamp_symbol(symbol: &mut skiff_artifact_model::PackageSymbolRef, abi: &str) {
        let owns_std = matches!(
            &symbol.package,
            PackageRefIr::PackageId { package_id } if package_id == HTTP_BOUNDARY_PACKAGE_ID
        ) || matches!(
            &symbol.package,
            PackageRefIr::Dependency { dependency_ref } if dependency_ref == "std"
        );
        if owns_std {
            symbol.package = PackageRefIr::PackageId {
                package_id: HTTP_BOUNDARY_PACKAGE_ID.to_string(),
            };
            symbol.abi_expectation = Some(abi.to_string());
        }
    }

    fn stamp_type(ty: &mut TypeRefIr, abi: &str) {
        match ty {
            TypeRefIr::PackageSymbol { symbol } => stamp_symbol(symbol, abi),
            TypeRefIr::Builtin { args, .. } | TypeRefIr::Union { items: args } => {
                for argument in args {
                    stamp_type(argument, abi);
                }
            }
            TypeRefIr::AppliedNominal { base, arguments } => {
                if let NominalTypeRefBaseIr::PackageSymbol { symbol } = base {
                    stamp_symbol(symbol, abi);
                }
                for argument in arguments {
                    stamp_type(argument, abi);
                }
            }
            TypeRefIr::Record { fields } => {
                for field in fields.values_mut() {
                    stamp_type(field, abi);
                }
            }
            TypeRefIr::Nullable { inner } => stamp_type(inner, abi),
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for parameter in params {
                    stamp_type(&mut parameter.ty, abi);
                }
                stamp_type(return_type, abi);
            }
            TypeRefIr::AnyInterface { interface } => {
                for argument in &mut interface.canonical_type_args {
                    stamp_type(argument, abi);
                }
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => {}
        }
    }
}
