use std::collections::BTreeMap;

use crate::{ContractLiteral, ContractTypeRef, PackageRefIr, PackageSymbolRef};

pub const HTTP_BOUNDARY_PACKAGE_ID: &str = "skiff.run/std";
pub const HTTP_REQUEST_TYPE: &str = "std.http.HttpRequest";
pub const HTTP_RESPONSE_TYPE: &str = "std.http.HttpResponse";
pub const HTTP_RESPONSE_STREAM_EVENT_TYPE: &str = "std.http.HttpResponseStreamEvent";

pub fn canonical_http_boundary_symbol(symbol: &PackageSymbolRef) -> Option<&str> {
    let PackageRefIr::PackageId { package_id } = &symbol.package else {
        return None;
    };
    if package_id != HTTP_BOUNDARY_PACKAGE_ID {
        return None;
    }
    canonical_http_boundary_type(&symbol.symbol_path).map(|_| symbol.symbol_path.as_str())
}

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
mod tests;
