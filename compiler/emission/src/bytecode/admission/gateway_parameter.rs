use std::collections::BTreeMap;

use skiff_artifact_identity::gateway_entry_identity;
use skiff_artifact_model::{
    http_boundary::HTTP_REQUEST_TYPE, DeploymentGatewayEntry, GatewayAdapterKind,
    GatewayAdapterSource, GatewayExternalErrorProjection, GatewayProtocolSurface, TypeRefIr,
};
use skiff_compiler_lowering::mir::MirUnit;

use super::server_stream::{exact_http_request_fields, exact_std_symbol_abi};
use crate::bytecode::inputs::canonical_function_key;

/// Untrusted transport of one exact compiler-projected gateway entry.
/// Admission rechecks the raw-HTTP request root before retaining any layout.
#[derive(Debug, Clone, PartialEq)]
pub struct GatewayParameterAuthority {
    entry: DeploymentGatewayEntry,
}

impl GatewayParameterAuthority {
    pub fn new(entry: DeploymentGatewayEntry) -> Self {
        Self { entry }
    }

    pub fn entry(&self) -> &DeploymentGatewayEntry {
        &self.entry
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DenseParameterMaterializationFact {
    pub(crate) slot: u32,
    pub(crate) ty: TypeRefIr,
    pub(crate) fields: BTreeMap<String, TypeRefIr>,
}

pub(super) fn analyze(
    units: &[MirUnit],
    transported: &[GatewayParameterAuthority],
) -> Result<BTreeMap<String, DenseParameterMaterializationFact>, String> {
    let mut functions = BTreeMap::new();
    for unit in units {
        for function in &unit.functions {
            functions
                .entry(&function.effect_summary_ref)
                .or_insert_with(Vec::new)
                .push((unit, function));
        }
    }

    let mut admitted = BTreeMap::new();
    for authority in transported {
        let entry = authority.entry();
        validate_raw_http_request_entry(entry)?;
        let handler = entry
            .handler
            .as_ref()
            .ok_or_else(|| "rawHttp gateway authority lacks an exact handler".to_string())?;
        let Some([(unit, function)]) = functions.get(handler).map(Vec::as_slice) else {
            return Err(format!(
                "rawHttp gateway handler {handler} does not name exactly one MIR function"
            ));
        };
        let [adapter_arg] = entry.adapter_plan.args.as_slice() else {
            return Err("rawHttp gateway must have one exact adapter argument".to_string());
        };
        let [parameter] = function.params.as_slice() else {
            return Err("rawHttp handler must have one exact parameter".to_string());
        };
        if adapter_arg.source != GatewayAdapterSource::HttpRequest
            || adapter_arg.param != parameter.name
        {
            return Err("gateway http.request argument differs from the MIR parameter".to_string());
        }
        let abi = exact_std_symbol_abi(unit, &parameter.ty, HTTP_REQUEST_TYPE)?;
        let fields = exact_http_request_fields(unit, &abi)?;
        let function_key = canonical_function_key(&unit.module_path, &function.symbol)
            .map_err(|error| error.to_string())?;
        let fact = DenseParameterMaterializationFact {
            slot: parameter.slot,
            ty: parameter.ty.clone(),
            fields,
        };
        match admitted.get(&function_key) {
            Some(existing) if existing != &fact => {
                return Err(format!(
                    "rawHttp gateway authorities disagree for handler {handler}"
                ));
            }
            Some(_) => {}
            None => {
                admitted.insert(function_key, fact);
            }
        }
    }
    Ok(admitted)
}

fn validate_raw_http_request_entry(entry: &DeploymentGatewayEntry) -> Result<(), String> {
    if gateway_entry_identity(&entry.protocol_surface).map_err(|error| error.to_string())?
        != entry.gateway_entry_identity
    {
        return Err("gateway entry identity differs from its typed surface".to_string());
    }
    let GatewayProtocolSurface::Http(http) = &entry.protocol_surface.protocol else {
        return Err("rawHttp parameter authority is not an HTTP gateway".to_string());
    };
    if http.adapter_kind != GatewayAdapterKind::RawHttp
        || http.external_sources.as_slice() != [GatewayAdapterSource::HttpRequest]
        || http.request_body_schema.is_some()
        || entry.protocol_surface.external_error_projection
            != GatewayExternalErrorProjection::FIXED_V1
        || entry.adapter_plan.kind != GatewayAdapterKind::RawHttp
    {
        return Err("gateway entry is not an exact rawHttp HttpRequest surface".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{http_boundary::HTTP_BOUNDARY_PACKAGE_ID, TypeRefIr};

    use super::*;

    const UNUSED_REQUEST_SOURCE: &str = r#"
import std

function consume(
  request: std.http.HttpRequest
) -> Stream<std.http.HttpResponseStreamEvent> {
  emit({ tag: "end" })
  return null
}
"#;

    #[test]
    fn exact_raw_http_root_is_admitted_without_any_field_access() {
        let (units, stream) =
            super::super::server_stream::tests::fixture_for_source(UNUSED_REQUEST_SOURCE);
        let facts = analyze(
            &units,
            &[GatewayParameterAuthority::new(stream.entry().clone())],
        )
        .expect("canonical rawHttp root is exact without GetDenseField authority");
        let fact = &facts["main::consume"];
        assert_eq!(fact.slot, 0);
        assert_eq!(
            fact.fields.keys().map(String::as_str).collect::<Vec<_>>(),
            ["body", "headers", "method", "path", "query", "url"]
        );
    }

    #[test]
    fn raw_http_root_rejects_wrong_nominal_abi_and_fields() {
        let (units, stream) =
            super::super::server_stream::tests::fixture_for_source(UNUSED_REQUEST_SOURCE);
        let authority = GatewayParameterAuthority::new(stream.entry().clone());

        let mut wrong_nominal = units.clone();
        wrong_nominal[0].functions[0].params[0].ty = TypeRefIr::Record {
            fields: BTreeMap::new(),
        };
        assert!(analyze(&wrong_nominal, std::slice::from_ref(&authority)).is_err());

        let mut wrong_abi = units.clone();
        let TypeRefIr::PackageSymbol { symbol } = &mut wrong_abi[0].functions[0].params[0].ty
        else {
            panic!("fixture request is a package symbol")
        };
        symbol.abi_expectation = Some("sha256:wrong".to_string());
        assert!(analyze(&wrong_abi, std::slice::from_ref(&authority)).is_err());

        let mut wrong_fields = units;
        wrong_fields[0]
            .package_type_records
            .get_mut(&(
                HTTP_BOUNDARY_PACKAGE_ID.to_string(),
                HTTP_REQUEST_TYPE.to_string(),
            ))
            .unwrap()
            .remove("path");
        assert!(analyze(&wrong_fields, &[authority]).is_err());
    }
}
