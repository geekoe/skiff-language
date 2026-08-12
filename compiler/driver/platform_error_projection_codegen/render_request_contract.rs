use std::{collections::BTreeSet, fmt::Write as _};

use skiff_artifact_model::{LiteralIr, NamedUnionBranchIr, TypeDescriptorIr, TypeRefIr};

use super::{
    fingerprint::FingerprintedCatalog,
    rust_names::{pascal_identifier, projection_rust_names, snake_identifier},
    PlatformErrorProjectionCodegenError, GENERATED_HEADER,
};

pub(super) fn render_request_contract(
    catalog: &FingerprintedCatalog<'_>,
) -> Result<String, PlatformErrorProjectionCodegenError> {
    let names = projection_rust_names(catalog)?;
    let mut output = String::new();
    writeln!(output, "{GENERATED_HEADER}").unwrap();
    output.push_str(
        "\nuse std::fmt;\n\n\
         use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};\n\
         use skiff_artifact_model::platform_error_projection::{\n\
         \x20   platform_error_projection_descriptor, PlatformErrorProjectionKey,\n\
         };\n\n",
    );

    for (entry, name) in catalog.entries.iter().zip(&names) {
        let payload_name = format!("{name}Payload");
        match &entry.resolved.canonical_public_type_ir().descriptor {
            TypeDescriptorIr::Record { fields } => {
                render_record(&mut output, &payload_name, fields)?;
            }
            TypeDescriptorIr::Union { branches } => {
                render_named_union(&mut output, &payload_name, branches)?;
            }
            other => {
                return Err(PlatformErrorProjectionCodegenError::Render(format!(
                    "projection {} has unsupported top-level descriptor {other:?}",
                    entry.resolved.projection_key()
                )));
            }
        }
    }

    output.push_str(
        "#[derive(Debug, Clone, PartialEq)]\n\
         pub enum PlatformErrorProjectionPayload {\n",
    );
    for name in &names {
        let variant = format!("    {name}({name}Payload),");
        if variant.len() <= 100 {
            writeln!(output, "{variant}").unwrap();
        } else {
            writeln!(output, "    {name}(").unwrap();
            writeln!(output, "        {name}Payload,").unwrap();
            output.push_str("    ),\n");
        }
    }
    output.push_str("}\n\nimpl PlatformErrorProjectionPayload {\n    pub const fn key(&self) -> PlatformErrorProjectionKey {\n        match self {\n");
    for name in &names {
        let arm = format!("            Self::{name}(_) => PlatformErrorProjectionKey::{name},");
        if arm.len() <= 100 {
            writeln!(output, "{arm}").unwrap();
        } else {
            writeln!(output, "            Self::{name}(_) => {{").unwrap();
            writeln!(output, "                PlatformErrorProjectionKey::{name}").unwrap();
            output.push_str("            }\n");
        }
    }
    output.push_str("        }\n    }\n}\n\n");

    output.push_str(
         "#[derive(Debug, Clone, PartialEq, Eq)]\n\
         pub struct EncodedPlatformErrorProjectionPayload {\n\
         \x20   projection_key: PlatformErrorProjectionKey,\n\
         \x20   entry_fingerprint: &'static str,\n\
         \x20   canonical_payload: Vec<u8>,\n\
         }\n\n\
         impl EncodedPlatformErrorProjectionPayload {\n\
         \x20   pub const fn projection_key(&self) -> PlatformErrorProjectionKey {\n\
         \x20       self.projection_key\n\
         \x20   }\n\n\
         \x20   pub const fn entry_fingerprint(&self) -> &'static str {\n\
         \x20       self.entry_fingerprint\n\
         \x20   }\n\n\
         \x20   pub fn canonical_payload(&self) -> &[u8] {\n\
         \x20       &self.canonical_payload\n\
         \x20   }\n\n\
         \x20   pub fn into_canonical_payload(self) -> Vec<u8> {\n\
         \x20       self.canonical_payload\n\
         \x20   }\n\
         }\n\n\
         #[derive(Debug, Clone, PartialEq)]\n\
         pub enum PlatformErrorProjectionDecodeOutcome {\n\
         \x20   Known(PlatformErrorProjectionPayload),\n\
         \x20   UnknownValid,\n\
         }\n\n\
         #[derive(Debug, Clone, PartialEq, Eq)]\n\
         pub enum PlatformErrorProjectionCodecError {\n\
         \x20   Serialization {\n\
         \x20       projection_key: PlatformErrorProjectionKey,\n\
         \x20       message: String,\n\
         \x20   },\n\
         \x20   MalformedKnownPayload {\n\
         \x20       projection_key: PlatformErrorProjectionKey,\n\
         \x20       message: String,\n\
         \x20   },\n\
         \x20   NonCanonicalKnownPayload {\n\
         \x20       projection_key: PlatformErrorProjectionKey,\n\
         \x20   },\n\
         }\n\n\
         impl fmt::Display for PlatformErrorProjectionCodecError {\n\
         \x20   fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {\n\
         \x20       match self {\n\
         \x20           Self::Serialization {\n\
         \x20               projection_key,\n\
         \x20               message,\n\
         \x20           } => write!(\n\
         \x20               formatter,\n\
         \x20               \"failed to encode known platform projection {projection_key}: {message}\"\n\
         \x20           ),\n\
         \x20           Self::MalformedKnownPayload {\n\
         \x20               projection_key,\n\
         \x20               message,\n\
         \x20           } => write!(\n\
         \x20               formatter,\n\
         \x20               \"malformed known platform projection {projection_key}: {message}\"\n\
         \x20           ),\n\
         \x20           Self::NonCanonicalKnownPayload { projection_key } => write!(\n\
         \x20               formatter,\n\
         \x20               \"known platform projection {projection_key} payload is not canonical JSON\"\n\
         \x20           ),\n\
         \x20       }\n\
         \x20   }\n\
         }\n\n\
         impl std::error::Error for PlatformErrorProjectionCodecError {}\n\n",
    );

    output.push_str(
        "pub fn encode_platform_error_projection_payload(\n\
         \x20   payload: &PlatformErrorProjectionPayload,\n\
         ) -> Result<EncodedPlatformErrorProjectionPayload, PlatformErrorProjectionCodecError> {\n\
         \x20   let projection_key = payload.key();\n\
         \x20   let canonical_payload = match payload {\n",
    );
    for name in &names {
        writeln!(
            output,
            "        PlatformErrorProjectionPayload::{name}(payload) => {{\n            encode_typed_payload(projection_key, payload)?\n        }}"
        )
        .unwrap();
    }
    output.push_str(
        "    };\n\
         \x20   let descriptor = platform_error_projection_descriptor(projection_key);\n\
         \x20   Ok(EncodedPlatformErrorProjectionPayload {\n\
         \x20       projection_key,\n\
         \x20       entry_fingerprint: descriptor.entry_fingerprint(),\n\
         \x20       canonical_payload,\n\
         \x20   })\n\
         }\n\n",
    );

    output.push_str(
        "pub fn decode_platform_error_projection_payload(\n\
         \x20   projection_key: &str,\n\
         \x20   entry_fingerprint: &str,\n\
         \x20   raw_payload: &[u8],\n\
         ) -> Result<PlatformErrorProjectionDecodeOutcome, PlatformErrorProjectionCodecError> {\n\
         \x20   let Ok(key) = PlatformErrorProjectionKey::parse_strict(projection_key) else {\n\
         \x20       return Ok(PlatformErrorProjectionDecodeOutcome::UnknownValid);\n\
         \x20   };\n\
         \x20   let descriptor = platform_error_projection_descriptor(key);\n\
         \x20   if entry_fingerprint != descriptor.entry_fingerprint() {\n\
         \x20       return Ok(PlatformErrorProjectionDecodeOutcome::UnknownValid);\n\
         \x20   }\n\
         \x20   materialize_platform_error_projection_payload(key, raw_payload)\n\
         \x20       .map(PlatformErrorProjectionDecodeOutcome::Known)\n\
         }\n\n",
    );

    output.push_str(
        "fn materialize_platform_error_projection_payload(\n\
         \x20   projection_key: PlatformErrorProjectionKey,\n\
         \x20   raw_payload: &[u8],\n\
         ) -> Result<PlatformErrorProjectionPayload, PlatformErrorProjectionCodecError> {\n\
         \x20   match projection_key {\n",
    );
    for name in &names {
        writeln!(
            output,
            "        PlatformErrorProjectionKey::{name} => {{\n            decode_typed_payload(projection_key, raw_payload)\n                .map(PlatformErrorProjectionPayload::{name})\n        }}"
        )
        .unwrap();
    }
    output.push_str("    }\n}\n\n");

    output.push_str(
        "fn encode_typed_payload<T: Serialize>(\n\
         \x20   projection_key: PlatformErrorProjectionKey,\n\
         \x20   payload: &T,\n\
         ) -> Result<Vec<u8>, PlatformErrorProjectionCodecError> {\n\
         \x20   canonical_json_bytes(payload).map_err(|message| {\n\
         \x20       PlatformErrorProjectionCodecError::Serialization {\n\
         \x20           projection_key,\n\
         \x20           message,\n\
         \x20       }\n\
         \x20   })\n\
         }\n\n\
         fn decode_typed_payload<T: DeserializeOwned + Serialize>(\n\
         \x20   projection_key: PlatformErrorProjectionKey,\n\
         \x20   raw_payload: &[u8],\n\
         ) -> Result<T, PlatformErrorProjectionCodecError> {\n\
         \x20   let payload = serde_json::from_slice::<T>(raw_payload).map_err(|error| {\n\
         \x20       PlatformErrorProjectionCodecError::MalformedKnownPayload {\n\
         \x20           projection_key,\n\
         \x20           message: error.to_string(),\n\
         \x20       }\n\
         \x20   })?;\n\
         \x20   let canonical = encode_typed_payload(projection_key, &payload)?;\n\
         \x20   if canonical != raw_payload {\n\
         \x20       return Err(PlatformErrorProjectionCodecError::NonCanonicalKnownPayload { projection_key });\n\
         \x20   }\n\
         \x20   Ok(payload)\n\
         }\n\n\
         fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>\n\
         where\n\
         \x20   D: Deserializer<'de>,\n\
         \x20   T: Deserialize<'de>,\n\
         {\n\
         \x20   Option::<T>::deserialize(deserializer)\n\
         }\n\n\
         fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {\n\
         \x20   skiff_canonical_json::canonical_json_bytes(value).map_err(|error| error.to_string())\n\
         }\n",
    );
    Ok(output)
}

fn render_record(
    output: &mut String,
    payload_name: &str,
    fields: &std::collections::BTreeMap<String, TypeRefIr>,
) -> Result<(), PlatformErrorProjectionCodegenError> {
    output.push_str(
        "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n\
         #[serde(deny_unknown_fields)]\n",
    );
    if fields.is_empty() {
        writeln!(output, "pub struct {payload_name} {{}}\n").unwrap();
        return Ok(());
    }
    writeln!(output, "pub struct {payload_name} {{").unwrap();
    render_fields(
        output,
        fields,
        None,
        "    ",
        RenderedFieldVisibility::Public,
    )?;
    output.push_str("}\n\n");
    Ok(())
}

fn render_named_union(
    output: &mut String,
    payload_name: &str,
    branches: &[NamedUnionBranchIr],
) -> Result<(), PlatformErrorProjectionCodegenError> {
    let discriminator = branches
        .first()
        .and_then(|branch| match branch {
            NamedUnionBranchIr::SyntheticDiscriminator {
                discriminator_field,
                ..
            } => Some(discriminator_field.as_str()),
            _ => None,
        })
        .ok_or_else(|| {
            PlatformErrorProjectionCodegenError::Render(format!(
                "named union {payload_name} has no synthetic-discriminator branch"
            ))
        })?;
    let mut variants = BTreeSet::new();
    output.push_str("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n");
    writeln!(
        output,
        "#[serde(tag = {discriminator:?}, deny_unknown_fields)]"
    )
    .unwrap();
    writeln!(output, "pub enum {payload_name} {{").unwrap();
    for branch in branches {
        let NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type,
            discriminator_field,
            discriminator_value,
        } = branch
        else {
            return Err(PlatformErrorProjectionCodegenError::Render(format!(
                "named union {payload_name} contains a non-synthetic branch"
            )));
        };
        if discriminator_field != discriminator {
            return Err(PlatformErrorProjectionCodegenError::Render(format!(
                "named union {payload_name} mixes discriminator fields"
            )));
        }
        let TypeRefIr::Record { fields } = payload_type else {
            return Err(PlatformErrorProjectionCodegenError::Render(format!(
                "named union {payload_name} branch payload is not a record"
            )));
        };
        validate_discriminator_literal(fields, discriminator_field, discriminator_value)?;
        let variant = pascal_identifier(discriminator_value);
        if !variants.insert(variant.clone()) {
            return Err(PlatformErrorProjectionCodegenError::Render(format!(
                "named union {payload_name} has colliding Rust variant {variant}"
            )));
        }
        writeln!(output, "    #[serde(rename = {discriminator_value:?})]").unwrap();
        let remaining = fields.len() - 1;
        if remaining == 0 {
            writeln!(output, "    {variant},").unwrap();
        } else {
            writeln!(output, "    {variant} {{").unwrap();
            render_fields(
                output,
                fields,
                Some(discriminator_field),
                "        ",
                RenderedFieldVisibility::Inherited,
            )?;
            output.push_str("    },\n");
        }
    }
    output.push_str("}\n\n");
    Ok(())
}

fn validate_discriminator_literal(
    fields: &std::collections::BTreeMap<String, TypeRefIr>,
    discriminator_field: &str,
    discriminator_value: &str,
) -> Result<(), PlatformErrorProjectionCodegenError> {
    let expected = TypeRefIr::Literal {
        value: LiteralIr::String {
            value: discriminator_value.to_owned(),
        },
    };
    if fields.get(discriminator_field) != Some(&expected) {
        return Err(PlatformErrorProjectionCodegenError::Render(format!(
            "discriminator {discriminator_field} does not equal literal {discriminator_value:?}"
        )));
    }
    Ok(())
}

fn render_fields(
    output: &mut String,
    fields: &std::collections::BTreeMap<String, TypeRefIr>,
    skipped_field: Option<&str>,
    indentation: &str,
    visibility: RenderedFieldVisibility,
) -> Result<(), PlatformErrorProjectionCodegenError> {
    let visibility = match visibility {
        RenderedFieldVisibility::Public => "pub ",
        RenderedFieldVisibility::Inherited => "",
    };
    let mut rust_fields = BTreeSet::new();
    for (field, ty) in fields {
        if skipped_field == Some(field.as_str()) {
            continue;
        }
        let rust_field = snake_identifier(field);
        if !rust_fields.insert(rust_field.clone()) {
            return Err(PlatformErrorProjectionCodegenError::Render(format!(
                "fields collide at generated Rust identifier {rust_field}"
            )));
        }
        writeln!(output, "{indentation}#[serde(rename = {field:?})]").unwrap();
        if matches!(ty, TypeRefIr::Nullable { .. }) {
            writeln!(
                output,
                "{indentation}#[serde(deserialize_with = \"deserialize_required_nullable\")]"
            )
            .unwrap();
        }
        writeln!(
            output,
            "{indentation}{visibility}{rust_field}: {},",
            rust_type(ty)?
        )
        .unwrap();
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderedFieldVisibility {
    Public,
    Inherited,
}

fn rust_type(ty: &TypeRefIr) -> Result<String, PlatformErrorProjectionCodegenError> {
    match ty {
        TypeRefIr::Builtin { name, args } if args.is_empty() => match name.as_str() {
            "string" => Ok("String".to_owned()),
            "integer" => Ok("i64".to_owned()),
            "number" => Ok("f64".to_owned()),
            "bool" | "boolean" => Ok("bool".to_owned()),
            "Json" => Ok("serde_json::Value".to_owned()),
            _ => Err(PlatformErrorProjectionCodegenError::Render(format!(
                "unsupported projection builtin {name}"
            ))),
        },
        TypeRefIr::Nullable { inner } => Ok(format!("Option<{}>", rust_type(inner)?)),
        other => Err(PlatformErrorProjectionCodegenError::Render(format!(
            "unsupported projection field type {other:?}"
        ))),
    }
}
