use std::collections::BTreeMap;

use crate::{ContractLiteral, ContractTypeRef};

pub const HTTP_REQUEST_TYPE: &str = "std.http.HttpRequest";
pub const HTTP_RESPONSE_TYPE: &str = "std.http.HttpResponse";
pub const HTTP_RESPONSE_STREAM_EVENT_TYPE: &str = "std.http.HttpResponseStreamEvent";

pub fn canonical_http_boundary_type(name: &str) -> Option<ContractTypeRef> {
    match name {
        HTTP_REQUEST_TYPE => Some(record([
            ("method", builtin("string")),
            ("url", builtin("string")),
            ("path", builtin("string")),
            ("query", array(name_value())),
            ("headers", array(name_value())),
            ("body", builtin("bytes")),
        ])),
        HTTP_RESPONSE_TYPE => Some(record([
            ("status", builtin("integer")),
            ("headers", array(name_value())),
            ("body", builtin("bytes")),
        ])),
        HTTP_RESPONSE_STREAM_EVENT_TYPE => Some(ContractTypeRef::StructuralUnion {
            variants: vec![
                record([
                    ("tag", literal("start")),
                    ("status", builtin("integer")),
                    ("headers", array(name_value())),
                ]),
                record([("tag", literal("chunk")), ("value", builtin("bytes"))]),
                record([("tag", literal("end"))]),
            ],
        }),
        _ => None,
    }
}

fn name_value() -> ContractTypeRef {
    record([("name", builtin("string")), ("value", builtin("string"))])
}

fn record<const N: usize>(fields: [(&str, ContractTypeRef); N]) -> ContractTypeRef {
    ContractTypeRef::Record {
        fields: fields
            .into_iter()
            .map(|(name, ty)| (name.to_string(), ty))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn array(item: ContractTypeRef) -> ContractTypeRef {
    ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![item],
    }
}

fn builtin(name: &str) -> ContractTypeRef {
    ContractTypeRef::builtin(name)
}

fn literal(value: &str) -> ContractTypeRef {
    ContractTypeRef::Literal {
        value: ContractLiteral::String {
            value: value.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_http_shapes_are_closed_and_exact() {
        let ContractTypeRef::Record { fields } =
            canonical_http_boundary_type(HTTP_REQUEST_TYPE).expect("request shape")
        else {
            panic!("request must be a record")
        };
        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["body", "headers", "method", "path", "query", "url"]
        );

        let ContractTypeRef::StructuralUnion { variants } =
            canonical_http_boundary_type(HTTP_RESPONSE_STREAM_EVENT_TYPE).expect("stream shape")
        else {
            panic!("stream event must be a union")
        };
        assert_eq!(variants.len(), 3);
        assert!(canonical_http_boundary_type("std.http.HttpClientRequest").is_none());
    }
}
