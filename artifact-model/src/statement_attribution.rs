//! Canonical source-event attribution and package-owned manifest identity.

mod identity;
mod model;

pub use identity::{
    derive_bytecode_statement_manifest_identity, validate_bytecode_statement_manifest_identity,
    validate_bytecode_statement_manifest_identity_lexical, BytecodeFunctionStatementManifest,
    BytecodeStatementManifestIdentity, StatementManifestIdentityError,
    BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX, BYTECODE_STATEMENT_MANIFEST_SCHEMA_MARKER,
};
pub use model::{
    validate_statement_entries_canonical, StatementAttributionClass, StatementAttributionId,
    StatementEntry, StatementEntryValidationError,
};

#[cfg(test)]
mod tests;
