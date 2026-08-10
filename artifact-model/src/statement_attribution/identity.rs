use std::fmt;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use sha2::Digest;

use crate::{validate_statement_entries_canonical, BytecodeFunctionOrigin, StatementEntry};

pub const BYTECODE_STATEMENT_MANIFEST_SCHEMA_MARKER: &str = "skiff-bytecode-statement-manifest-v1";
pub const BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX: &str =
    "skiff-bytecode-statement-manifest-v1:sha256";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BytecodeStatementManifestIdentity(String);

impl BytecodeStatementManifestIdentity {
    pub fn parse(value: impl Into<String>) -> Result<Self, StatementManifestIdentityError> {
        let value = value.into();
        validate_bytecode_statement_manifest_identity_lexical_value(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BytecodeStatementManifestIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BytecodeStatementManifestIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytecodeFunctionStatementManifest {
    pub origin: BytecodeFunctionOrigin,
    pub statement_entries: Vec<StatementEntry>,
}

impl BytecodeFunctionStatementManifest {
    pub fn new(origin: BytecodeFunctionOrigin, statement_entries: Vec<StatementEntry>) -> Self {
        Self {
            origin,
            statement_entries,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementManifestIdentityError {
    message: String,
}

impl StatementManifestIdentityError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StatementManifestIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StatementManifestIdentityError {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestIdentityInput<'a> {
    schema: &'static str,
    package_id: &'a str,
    functions: Vec<FunctionProjection<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionProjection<'a> {
    origin: &'a BytecodeFunctionOrigin,
    statement_entries: &'a [StatementEntry],
}

pub fn derive_bytecode_statement_manifest_identity(
    package_id: &str,
    functions: &[BytecodeFunctionStatementManifest],
) -> Result<BytecodeStatementManifestIdentity, StatementManifestIdentityError> {
    validate_manifest_input(package_id, functions)?;
    derive_manifest_identity_from_projection(package_id, functions)
}

fn derive_manifest_identity_from_projection(
    package_id: &str,
    functions: &[BytecodeFunctionStatementManifest],
) -> Result<BytecodeStatementManifestIdentity, StatementManifestIdentityError> {
    let input = ManifestIdentityInput {
        schema: BYTECODE_STATEMENT_MANIFEST_SCHEMA_MARKER,
        package_id,
        functions: functions
            .iter()
            .map(|function| FunctionProjection {
                origin: &function.origin,
                statement_entries: &function.statement_entries,
            })
            .collect(),
    };
    let bytes = skiff_canonical_json::canonical_json_bytes(&input).map_err(|error| {
        StatementManifestIdentityError::new(format!(
            "failed to canonicalize bytecode statement manifest: {error}"
        ))
    })?;
    BytecodeStatementManifestIdentity::parse(format!(
        "{BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX}:{}",
        hex::encode(sha2::Sha256::digest(bytes))
    ))
}

#[cfg(test)]
pub(super) fn derive_manifest_identity_from_projection_for_test(
    package_id: &str,
    functions: &[BytecodeFunctionStatementManifest],
) -> Result<BytecodeStatementManifestIdentity, StatementManifestIdentityError> {
    derive_manifest_identity_from_projection(package_id, functions)
}

pub fn validate_bytecode_statement_manifest_identity(
    package_id: &str,
    functions: &[BytecodeFunctionStatementManifest],
    declared: &BytecodeStatementManifestIdentity,
) -> Result<(), StatementManifestIdentityError> {
    let expected = derive_bytecode_statement_manifest_identity(package_id, functions)?;
    if &expected != declared {
        return invalid(format!(
            "bytecode statement manifest identity is {declared}, expected {expected}"
        ));
    }
    Ok(())
}

pub fn validate_bytecode_statement_manifest_identity_lexical(
    declared: &BytecodeStatementManifestIdentity,
) -> Result<(), StatementManifestIdentityError> {
    validate_bytecode_statement_manifest_identity_lexical_value(declared.as_str())
}

fn validate_manifest_input(
    package_id: &str,
    functions: &[BytecodeFunctionStatementManifest],
) -> Result<(), StatementManifestIdentityError> {
    if package_id.is_empty()
        || package_id
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return invalid(
            "packageId must be non-empty and contain no whitespace or control characters",
        );
    }
    let mut previous_origin = None;
    for (index, function) in functions.iter().enumerate() {
        if previous_origin.is_some_and(|previous| previous >= &function.origin) {
            return invalid(format!(
                "statement manifest function {index} is not strictly ordered by origin"
            ));
        }
        validate_statement_entries_canonical(&function.statement_entries).map_err(|error| {
            StatementManifestIdentityError::new(format!(
                "statement manifest function {index} is not canonical: {error}"
            ))
        })?;
        previous_origin = Some(&function.origin);
    }
    Ok(())
}

fn validate_bytecode_statement_manifest_identity_lexical_value(
    value: &str,
) -> Result<(), StatementManifestIdentityError> {
    let prefix = format!("{BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX}:");
    let Some(digest) = value.strip_prefix(&prefix) else {
        return invalid(format!(
            "bytecode statement manifest identity must start with {prefix}"
        ));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return invalid(
            "bytecode statement manifest identity digest must be 64 lowercase hexadecimal characters",
        );
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, StatementManifestIdentityError> {
    Err(StatementManifestIdentityError::new(message))
}
