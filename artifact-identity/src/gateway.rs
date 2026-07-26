use std::collections::BTreeSet;

use serde::Serialize;
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterSource, GatewayDispatchMode, GatewayEntryIdentity,
    GatewayEntryProtocolSurface, GatewayExternalErrorProjection, GatewayExternalSchema,
    GatewayHttpProtocolSurface, GatewayProtocolSurface,
};

use crate::{
    framing::{canonical_ir_bytes, framed_identity, sha256_hex},
    ArtifactIdentityError, Result, GATEWAY_ENTRY_IDENTITY_PREFIX,
    GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER,
};

/// Complete, implementation-free preimage of `GatewayEntryIdentity`.
///
/// Its private fields make selectors, owner-local keys, callable targets,
/// parameter names, package/build facts and runtime codec plans impossible to
/// add at construction sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayEntryIdentityProjection {
    schema: &'static str,
    surface: GatewayEntryProtocolSurface,
}

impl GatewayEntryIdentityProjection {
    pub fn surface(&self) -> &GatewayEntryProtocolSurface {
        &self.surface
    }
}

/// Producer-side canonicalization.
///
/// Loaded artifacts must call `validate_gateway_entry_protocol_surface`,
/// which compares the loaded value with this output and rejects rather than
/// silently repairing non-canonical collections.
pub fn normalize_gateway_entry_protocol_surface(
    mut surface: GatewayEntryProtocolSurface,
) -> Result<GatewayEntryProtocolSurface> {
    surface.protocol = match surface.protocol {
        GatewayProtocolSurface::Http(http) => {
            GatewayProtocolSurface::Http(normalize_http_surface(http)?)
        }
    };
    validate_surface_semantics(&surface)?;
    Ok(surface)
}

pub fn normalize_gateway_external_schema(
    schema: GatewayExternalSchema,
) -> Result<GatewayExternalSchema> {
    normalize_schema(schema, "externalSchema")
}

/// Artifact-reader boundary validation. Non-canonical sequence ordering,
/// redundant unions/nullability and duplicate source selection are rejected.
pub fn validate_gateway_entry_protocol_surface(
    surface: &GatewayEntryProtocolSurface,
) -> Result<()> {
    let normalized = normalize_gateway_entry_protocol_surface(surface.clone())?;
    if &normalized != surface {
        return invalid_surface(
            "surface collections and external schemas must already be canonical",
        );
    }
    Ok(())
}

pub fn gateway_entry_identity_projection(
    surface: &GatewayEntryProtocolSurface,
) -> Result<GatewayEntryIdentityProjection> {
    validate_gateway_entry_protocol_surface(surface)?;
    Ok(GatewayEntryIdentityProjection {
        schema: GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER,
        surface: surface.clone(),
    })
}

pub fn canonical_gateway_entry_identity_bytes(
    surface: &GatewayEntryProtocolSurface,
) -> Result<Vec<u8>> {
    canonical_ir_bytes(
        &gateway_entry_identity_projection(surface)?,
        ArtifactIdentityError::SerializeGatewayEntryIdentity,
    )
}

pub fn gateway_entry_identity(
    surface: &GatewayEntryProtocolSurface,
) -> Result<GatewayEntryIdentity> {
    let bytes = canonical_gateway_entry_identity_bytes(surface)?;
    let identity = framed_identity(GATEWAY_ENTRY_IDENTITY_PREFIX, &sha256_hex(&bytes));
    GatewayEntryIdentity::parse(&identity).map_err(|_| {
        ArtifactIdentityError::InvalidGatewayEntryIdentity {
            identity: identity.clone(),
        }
    })
}

pub fn gateway_entry_identity_hash(identity: &str) -> Result<&str> {
    identity
        .strip_prefix(GATEWAY_ENTRY_IDENTITY_PREFIX)
        .and_then(|suffix| suffix.strip_prefix(':'))
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
        .ok_or_else(|| ArtifactIdentityError::InvalidGatewayEntryIdentity {
            identity: identity.to_string(),
        })
}

fn normalize_http_surface(
    mut surface: GatewayHttpProtocolSurface,
) -> Result<GatewayHttpProtocolSurface> {
    normalize_sources(&mut surface.external_sources);
    surface.request_body_schema = surface
        .request_body_schema
        .map(|schema| normalize_schema(schema, "protocol.http.requestBodySchema"))
        .transpose()?;
    surface.response_schema = surface
        .response_schema
        .map(|schema| normalize_schema(schema, "protocol.http.responseSchema"))
        .transpose()?;
    surface.stream_item_schema = surface
        .stream_item_schema
        .map(|schema| normalize_schema(schema, "protocol.http.streamItemSchema"))
        .transpose()?;
    Ok(surface)
}

fn normalize_sources(sources: &mut Vec<GatewayAdapterSource>) {
    sources.sort_by_key(|source| source.wire_name());
    sources.dedup();
}

fn normalize_schema(schema: GatewayExternalSchema, path: &str) -> Result<GatewayExternalSchema> {
    match schema {
        GatewayExternalSchema::Array { items } => Ok(GatewayExternalSchema::Array {
            items: Box::new(normalize_schema(*items, &format!("{path}.items"))?),
        }),
        GatewayExternalSchema::Record {
            fields,
            mut required,
        } => {
            let mut normalized_fields = fields;
            for (name, field) in &mut normalized_fields {
                validate_schema_field_name(name, &format!("{path}.fields"))?;
                *field = normalize_schema(field.clone(), &format!("{path}.fields[{name}]"))?;
            }
            let mut seen = BTreeSet::new();
            for name in &required {
                validate_schema_field_name(name, &format!("{path}.required"))?;
                if !normalized_fields.contains_key(name) {
                    return invalid_surface(format!(
                        "{path}.required field {name:?} is not present in fields"
                    ));
                }
                if !seen.insert(name.as_str()) {
                    return invalid_surface(format!(
                        "{path}.required contains duplicate field {name:?}"
                    ));
                }
            }
            required.sort();
            Ok(GatewayExternalSchema::Record {
                fields: normalized_fields,
                required,
            })
        }
        GatewayExternalSchema::ClosedUnion { branches } => normalize_union(branches, path),
        GatewayExternalSchema::Nullable { inner } => {
            let inner = normalize_schema(*inner, &format!("{path}.inner"))?;
            match inner {
                GatewayExternalSchema::Null => Ok(GatewayExternalSchema::Null),
                GatewayExternalSchema::Nullable { .. } => Ok(inner),
                _ => Ok(GatewayExternalSchema::Nullable {
                    inner: Box::new(inner),
                }),
            }
        }
        GatewayExternalSchema::Null
        | GatewayExternalSchema::String
        | GatewayExternalSchema::Number
        | GatewayExternalSchema::Integer
        | GatewayExternalSchema::Boolean
        | GatewayExternalSchema::Bytes
        | GatewayExternalSchema::StringLiteral { .. } => Ok(schema),
    }
}

fn normalize_union(
    branches: Vec<GatewayExternalSchema>,
    path: &str,
) -> Result<GatewayExternalSchema> {
    if branches.is_empty() {
        return invalid_surface(format!("{path}.branches must not be empty"));
    }

    let mut members = Vec::new();
    let mut has_null = false;
    for (index, branch) in branches.into_iter().enumerate() {
        let branch = normalize_schema(branch, &format!("{path}.branches[{index}]"))?;
        collect_union_member(branch, path, &mut members, &mut has_null)?;
    }
    let mut keyed_members = members
        .into_iter()
        .map(|member| Ok((schema_sort_key(&member)?, member)))
        .collect::<Result<Vec<_>>>()?;
    keyed_members.sort_by(|left, right| left.0.cmp(&right.0));
    if keyed_members.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return invalid_surface(format!("{path}.branches contains a duplicate branch"));
    }
    let mut members = keyed_members
        .into_iter()
        .map(|(_, member)| member)
        .collect::<Vec<_>>();

    let base = match members.len() {
        0 if has_null => return Ok(GatewayExternalSchema::Null),
        0 => return invalid_surface(format!("{path}.branches must not be empty")),
        1 => members.pop().expect("one member"),
        _ => GatewayExternalSchema::ClosedUnion { branches: members },
    };
    if has_null {
        Ok(GatewayExternalSchema::Nullable {
            inner: Box::new(base),
        })
    } else {
        Ok(base)
    }
}

fn collect_union_member(
    branch: GatewayExternalSchema,
    path: &str,
    members: &mut Vec<GatewayExternalSchema>,
    has_null: &mut bool,
) -> Result<()> {
    match branch {
        GatewayExternalSchema::Null => {
            if *has_null {
                return invalid_surface(format!("{path}.branches contains duplicate nullability"));
            }
            *has_null = true;
        }
        GatewayExternalSchema::Nullable { inner } => {
            if *has_null {
                return invalid_surface(format!("{path}.branches contains duplicate nullability"));
            }
            *has_null = true;
            collect_non_null_union_member(*inner, members);
        }
        GatewayExternalSchema::ClosedUnion { branches } => members.extend(branches),
        member => members.push(member),
    }
    Ok(())
}

fn collect_non_null_union_member(
    branch: GatewayExternalSchema,
    members: &mut Vec<GatewayExternalSchema>,
) {
    match branch {
        GatewayExternalSchema::ClosedUnion { branches } => members.extend(branches),
        member => members.push(member),
    }
}

fn schema_sort_key(schema: &GatewayExternalSchema) -> Result<Vec<u8>> {
    canonical_ir_bytes(schema, ArtifactIdentityError::SerializeGatewayEntryIdentity)
}

fn validate_schema_field_name(name: &str, path: &str) -> Result<()> {
    if name.is_empty() || name != name.trim() || name.chars().any(char::is_control) {
        return invalid_surface(format!(
            "{path} field name {name:?} must be non-empty, trimmed and contain no control characters"
        ));
    }
    Ok(())
}

fn validate_surface_semantics(surface: &GatewayEntryProtocolSurface) -> Result<()> {
    if surface.external_error_projection != GatewayExternalErrorProjection::FIXED_V1 {
        return invalid_surface("externalErrorProjection must be fixed v1");
    }
    match &surface.protocol {
        GatewayProtocolSurface::Http(http) => validate_http_surface(http),
    }
}

fn validate_http_surface(surface: &GatewayHttpProtocolSurface) -> Result<()> {
    for source in &surface.external_sources {
        if !source.is_external_protocol_source() {
            return invalid_surface(format!(
                "HTTP protocol surface cannot contain internal source {}",
                source.wire_name()
            ));
        }
    }

    match surface.adapter_kind {
        GatewayAdapterKind::TypedJson => {
            if surface.external_sources.iter().any(|source| {
                !matches!(
                    source,
                    GatewayAdapterSource::HttpRequest | GatewayAdapterSource::HttpBody
                )
            }) {
                return invalid_surface("typed HTTP surface contains a non-HTTP source");
            }
            if !surface
                .external_sources
                .contains(&GatewayAdapterSource::HttpBody)
            {
                return invalid_surface("typed HTTP surface must select http.body");
            }
            if surface.request_body_schema.is_none() {
                return invalid_surface("typed HTTP surface requires requestBodySchema");
            }
            match surface.dispatch_mode {
                GatewayDispatchMode::Unary => {
                    if surface.response_schema.is_none() {
                        return invalid_surface("typed unary HTTP surface requires responseSchema");
                    }
                    if surface.stream_item_schema.is_some() {
                        return invalid_surface(
                            "unary HTTP surface must not carry streamItemSchema",
                        );
                    }
                }
                GatewayDispatchMode::ServerStream => {
                    if surface.response_schema.is_some() {
                        return invalid_surface(
                            "typed server-stream HTTP surface uses streamItemSchema, not responseSchema",
                        );
                    }
                    if surface.stream_item_schema.is_none() {
                        return invalid_surface(
                            "server-stream HTTP surface requires streamItemSchema",
                        );
                    }
                }
            }
        }
        GatewayAdapterKind::RawHttp => {
            if surface.external_sources.as_slice() != [GatewayAdapterSource::HttpRequest] {
                return invalid_surface(
                    "raw HTTP surface must select exactly the external http.request source",
                );
            }
            if surface.request_body_schema.is_some() {
                return invalid_surface("raw HTTP surface must not carry requestBodySchema");
            }
            if surface.response_schema.is_some() {
                return invalid_surface("raw HTTP surface must not carry typed responseSchema");
            }
            match surface.dispatch_mode {
                GatewayDispatchMode::Unary if surface.stream_item_schema.is_some() => {
                    return invalid_surface("unary HTTP surface must not carry streamItemSchema");
                }
                GatewayDispatchMode::ServerStream if surface.stream_item_schema.is_none() => {
                    return invalid_surface("server-stream HTTP surface requires streamItemSchema");
                }
                GatewayDispatchMode::Unary | GatewayDispatchMode::ServerStream => {}
            }
        }
    }
    Ok(())
}

fn invalid_surface<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidGatewayEntryProtocolSurface {
        message: message.into(),
    })
}
