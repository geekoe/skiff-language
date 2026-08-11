use serde_json::Value;

use crate::{
    contract::RuntimeBoundaryContract,
    error::{Result, RuntimeError},
    json::reject_reserved_legacy_metadata_key,
    plan::{BoundaryConversionPlan, BoundaryDirection, BoundaryUse},
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, RuntimeObject, RuntimeObjectFields, RuntimeValue},
    runtime_value_graph::RuntimeValueGraph,
    type_descriptor::RuntimeTypePlan,
    value::bytes_payload,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBoundaryNameValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBoundaryRequestParts {
    pub method: String,
    pub url: String,
    pub path: String,
    pub query: Vec<HttpBoundaryNameValue>,
    pub headers: Vec<HttpBoundaryNameValue>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBoundaryResponseParts {
    pub status: u16,
    pub headers: Vec<HttpBoundaryNameValue>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpBoundaryResponseStreamEvent {
    Start {
        status: u16,
        headers: Vec<HttpBoundaryNameValue>,
    },
    Chunk(Vec<u8>),
    End,
}

#[allow(clippy::large_enum_variant)]
pub enum HttpBoundaryPlanInput<'a> {
    Borrowed(&'a BoundaryConversionPlan),
    Owned(BoundaryConversionPlan),
}

impl<'a> HttpBoundaryPlanInput<'a> {
    fn as_plan(&self) -> &BoundaryConversionPlan {
        match self {
            Self::Borrowed(plan) => plan,
            Self::Owned(plan) => plan,
        }
    }
}

pub trait IntoHttpBoundaryPlan<'a> {
    fn into_http_boundary_plan(
        self,
        use_case: BoundaryUse,
        direction: BoundaryDirection,
    ) -> HttpBoundaryPlanInput<'a>;
}

impl<'a> IntoHttpBoundaryPlan<'a> for &'a BoundaryConversionPlan {
    fn into_http_boundary_plan(
        self,
        _use_case: BoundaryUse,
        _direction: BoundaryDirection,
    ) -> HttpBoundaryPlanInput<'a> {
        HttpBoundaryPlanInput::Borrowed(self)
    }
}

impl<'a> IntoHttpBoundaryPlan<'a> for &'a RuntimeTypePlan {
    fn into_http_boundary_plan(
        self,
        use_case: BoundaryUse,
        direction: BoundaryDirection,
    ) -> HttpBoundaryPlanInput<'a> {
        HttpBoundaryPlanInput::Owned(RuntimeBoundaryContract::default().conversion_plan(
            self.clone(),
            use_case,
            direction,
        ))
    }
}

pub fn typed_json_body_decode_plan(expected_type: RuntimeTypePlan) -> BoundaryConversionPlan {
    RuntimeBoundaryContract::default().conversion_plan(
        expected_type,
        BoundaryUse::HttpRequest,
        BoundaryDirection::Decode,
    )
}

pub fn typed_json_response_encode_plan(expected_type: RuntimeTypePlan) -> BoundaryConversionPlan {
    RuntimeBoundaryContract::default().conversion_plan(
        expected_type,
        BoundaryUse::HttpResponse,
        BoundaryDirection::Encode,
    )
}

pub fn direct_http_request_coerce_plan(expected_type: RuntimeTypePlan) -> BoundaryConversionPlan {
    RuntimeBoundaryContract::default().conversion_plan(
        expected_type,
        BoundaryUse::HttpRequest,
        BoundaryDirection::Coerce,
    )
}

pub fn direct_http_response_coerce_plan(expected_type: RuntimeTypePlan) -> BoundaryConversionPlan {
    RuntimeBoundaryContract::default().conversion_plan(
        expected_type,
        BoundaryUse::HttpResponse,
        BoundaryDirection::Coerce,
    )
}

pub fn decode_typed_json_body(
    input: &str,
    plan: &BoundaryConversionPlan,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    ensure_http_plan(
        plan,
        BoundaryUse::HttpRequest,
        BoundaryDirection::Decode,
        "HTTP adapter body",
    )?;
    RuntimeBoundaryContract::default()
        .codec(plan, "HTTP adapter body")
        .decode_json_text(input, heap)
}

pub fn encode_typed_json_response(
    value: &RuntimeValue,
    plan: &BoundaryConversionPlan,
    heap: &mut RequestHeap,
) -> Result<String> {
    ensure_http_plan(
        plan,
        BoundaryUse::HttpResponse,
        BoundaryDirection::Encode,
        "HTTP adapter response",
    )?;
    RuntimeBoundaryContract::default()
        .codec(plan, "HTTP adapter response")
        .encode_json_text_value(value, heap)
}

pub fn direct_http_request_runtime_value<'a>(
    parts: &HttpBoundaryRequestParts,
    plan: impl IntoHttpBoundaryPlan<'a>,
    label: impl Into<String>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let plan = plan.into_http_boundary_plan(BoundaryUse::HttpRequest, BoundaryDirection::Coerce);
    let plan = plan.as_plan();
    ensure_http_plan(
        plan,
        BoundaryUse::HttpRequest,
        BoundaryDirection::Coerce,
        "HTTP request",
    )?;
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "method".to_string(),
        RuntimeValue::String(parts.method.clone()),
    );
    fields.insert("url".to_string(), RuntimeValue::String(parts.url.clone()));
    fields.insert("path".to_string(), RuntimeValue::String(parts.path.clone()));
    fields.insert(
        "query".to_string(),
        RuntimeValue::Heap(alloc_name_value_array(heap, &parts.query)?),
    );
    fields.insert(
        "headers".to_string(),
        RuntimeValue::Heap(alloc_name_value_array(heap, &parts.headers)?),
    );
    let body = heap.alloc_bytes(parts.body.clone())?;
    fields.insert("body".to_string(), RuntimeValue::Heap(body));
    let value = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .map(RuntimeValue::Heap)?;
    RuntimeBoundaryContract::default()
        .codec(plan, label.into())
        .coerce_runtime_value(&value, heap)
}

pub fn direct_http_response_from_runtime_value<'a>(
    value: &RuntimeValue,
    plan: impl IntoHttpBoundaryPlan<'a>,
    label: impl Into<String>,
    heap: &mut RequestHeap,
) -> Result<HttpBoundaryResponseParts> {
    let plan = plan.into_http_boundary_plan(BoundaryUse::HttpResponse, BoundaryDirection::Coerce);
    let plan = plan.as_plan();
    ensure_http_plan(
        plan,
        BoundaryUse::HttpResponse,
        BoundaryDirection::Coerce,
        "HTTP response",
    )?;
    let coerced = RuntimeBoundaryContract::default()
        .codec(plan, label.into())
        .coerce_runtime_value(value, heap)?;
    let object = runtime_object_fields_for_http(&coerced, heap, "HttpResponse")?;
    let status = runtime_integer_field(object, "status")?;
    if !(100..=599).contains(&status) {
        return Err(RuntimeError::Decode(
            "HttpResponse.status must be an integer between 100 and 599".to_string(),
        ));
    }
    let headers = runtime_name_value_array_field(object, "headers", heap)?;
    let body = runtime_bytes_field(object, "body", heap)?.to_vec();
    Ok(HttpBoundaryResponseParts {
        status: status as u16,
        headers,
        body,
    })
}

fn ensure_http_plan(
    plan: &BoundaryConversionPlan,
    use_case: BoundaryUse,
    direction: BoundaryDirection,
    context: &str,
) -> Result<()> {
    if plan.use_case() == use_case && plan.direction() == direction {
        return Ok(());
    }
    Err(RuntimeError::InvalidArtifact(format!(
        "{context} boundary expected {use_case:?}/{direction:?} conversion plan, got {:?}/{:?}",
        plan.use_case(),
        plan.direction()
    )))
}

pub fn http_response_stream_event_from_wire(
    value: &Value,
) -> Result<HttpBoundaryResponseStreamEvent> {
    let object = value.as_object().ok_or_else(|| {
        RuntimeError::Decode("HttpResponseStreamEvent must be an object".to_string())
    })?;
    let tag = object.get("tag").and_then(Value::as_str).ok_or_else(|| {
        RuntimeError::Decode("HttpResponseStreamEvent.tag must be a string".to_string())
    })?;
    match tag {
        "start" => {
            require_exact_fields(object, &["tag", "status", "headers"], "start")?;
            let status = value_integer_field(object, "status")?;
            if !(100..=599).contains(&status) {
                return Err(RuntimeError::Decode(
                    "HttpResponseStreamEvent.start.status must be an integer between 100 and 599"
                        .to_string(),
                ));
            }
            let headers = value_name_value_array_field(object.get("headers"), "headers")?;
            Ok(HttpBoundaryResponseStreamEvent::Start {
                status: status as u16,
                headers,
            })
        }
        "chunk" => {
            require_exact_fields(object, &["tag", "value"], "chunk")?;
            let value = object.get("value").ok_or_else(|| {
                RuntimeError::Decode(
                    "HttpResponseStreamEvent.chunk.value must be bytes".to_string(),
                )
            })?;
            let bytes = bytes_payload(value).ok_or_else(|| {
                RuntimeError::Decode(
                    "HttpResponseStreamEvent.chunk.value must be bytes".to_string(),
                )
            })?;
            Ok(HttpBoundaryResponseStreamEvent::Chunk(bytes))
        }
        "end" => {
            require_exact_fields(object, &["tag"], "end")?;
            Ok(HttpBoundaryResponseStreamEvent::End)
        }
        other => Err(RuntimeError::Decode(format!(
            "unsupported HttpResponseStreamEvent tag {other}"
        ))),
    }
}

fn require_exact_fields(
    object: &serde_json::Map<String, Value>,
    expected: &[&str],
    variant: &str,
) -> Result<()> {
    let mut actual = object.keys().map(String::as_str).collect::<Vec<_>>();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    if actual == expected {
        return Ok(());
    }
    Err(RuntimeError::Decode(format!(
        "HttpResponseStreamEvent.{variant} fields must be exactly {}",
        expected.join(", ")
    )))
}

fn alloc_name_value_array(
    heap: &mut RequestHeap,
    items: &[HttpBoundaryNameValue],
) -> Result<HeapHandle> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let mut fields = RuntimeObjectFields::new();
        fields.insert("name".to_string(), RuntimeValue::String(item.name.clone()));
        fields.insert(
            "value".to_string(),
            RuntimeValue::String(item.value.clone()),
        );
        let handle = heap.alloc_object(RuntimeObject::unshaped(fields))?;
        values.push(RuntimeValue::Heap(handle));
    }
    Ok(heap.alloc_array(values)?)
}

fn runtime_object_fields_for_http<'a>(
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
    context: &str,
) -> Result<&'a RuntimeObjectFields> {
    let fields = RuntimeValueGraph::new(heap)
        .object_fields_or_error(value, &format!("{context} must be an object"))?;
    for key in fields.keys() {
        reject_reserved_legacy_metadata_key(key)?;
    }
    Ok(fields)
}

fn runtime_integer_field(object: &RuntimeObjectFields, field: &str) -> Result<i64> {
    match object.get(field) {
        Some(RuntimeValue::Number(value))
            if value.is_finite() && value.fract() == 0.0 && *value >= 0.0 =>
        {
            Ok(*value as i64)
        }
        _ => Err(RuntimeError::Decode(format!(
            "HttpResponse.{field} must be an integer"
        ))),
    }
}

fn runtime_name_value_array_field(
    object: &RuntimeObjectFields,
    field: &str,
    heap: &RequestHeap,
) -> Result<Vec<HttpBoundaryNameValue>> {
    let Some(value) = object.get(field) else {
        return Err(RuntimeError::Decode(format!(
            "HttpResponse.{field} must be an array"
        )));
    };
    let items = RuntimeValueGraph::new(heap)
        .array_or_error(value, &format!("HttpResponse.{field} must be an array"))?;
    items
        .iter()
        .map(|item| {
            let fields = runtime_object_fields_for_http(item, heap, "HttpResponse.header")?;
            Ok(HttpBoundaryNameValue {
                name: runtime_string_field(fields, "name")?.to_string(),
                value: runtime_string_field(fields, "value")?.to_string(),
            })
        })
        .collect()
}

fn runtime_string_field<'a>(object: &'a RuntimeObjectFields, field: &str) -> Result<&'a str> {
    match object.get(field) {
        Some(RuntimeValue::String(value)) => Ok(value.as_str()),
        _ => Err(RuntimeError::Decode(format!(
            "HTTP {field} field must be a string"
        ))),
    }
}

fn runtime_bytes_field<'a>(
    object: &'a RuntimeObjectFields,
    field: &str,
    heap: &'a RequestHeap,
) -> Result<&'a [u8]> {
    let Some(value) = object.get(field) else {
        return Err(RuntimeError::Decode(format!(
            "HttpResponse.{field} must be bytes"
        )));
    };
    Ok(RuntimeValueGraph::new(heap)
        .bytes_or_error(value, &format!("HttpResponse.{field} must be bytes"))?)
}

fn value_integer_field(object: &serde_json::Map<String, Value>, field: &str) -> Result<i64> {
    match object.get(field).and_then(Value::as_i64) {
        Some(value) => Ok(value),
        None => Err(RuntimeError::Decode(format!(
            "HttpResponseStreamEvent.{field} must be an integer"
        ))),
    }
}

fn value_name_value_array_field(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<HttpBoundaryNameValue>> {
    let items = value.and_then(Value::as_array).ok_or_else(|| {
        RuntimeError::Decode(format!("HttpResponseStreamEvent.{field} must be an array"))
    })?;
    items
        .iter()
        .map(|item| {
            let object = item.as_object().ok_or_else(|| {
                RuntimeError::Decode("HttpResponseStreamEvent.header must be an object".to_string())
            })?;
            require_exact_fields(object, &["name", "value"], "header")?;
            Ok(HttpBoundaryNameValue {
                name: value_string_field(object, "name")?.to_string(),
                value: value_string_field(object, "value")?.to_string(),
            })
        })
        .collect()
}

fn value_string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeError::Decode(format!("HTTP {field} field must be a string")))
}

#[cfg(test)]
mod tests;
