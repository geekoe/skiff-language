use std::{borrow::Cow, error::Error};

/// Maximum number of structured attributes retained for one diagnostic.
pub const MAX_DIAGNOSTIC_ATTRIBUTES: usize = 8;

/// A low-cardinality, machine-readable diagnostic label.
///
/// Codes are static ASCII tokens. They describe diagnostics only and do not
/// grant a Skiff catch identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DiagnosticCode(&'static str);

impl DiagnosticCode {
    /// Creates a code when `value` is a 1..=128 byte ASCII token.
    pub const fn new(value: &'static str) -> Option<Self> {
        if is_valid_token(value, 128) {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// The key of one restricted diagnostic attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DiagnosticFieldKey(&'static str);

impl DiagnosticFieldKey {
    /// Creates a key when `value` is a 1..=64 byte ASCII token.
    pub const fn new(value: &'static str) -> Option<Self> {
        if is_valid_token(value, 64) {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// A bounded string-like diagnostic value declared in source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StaticDiagnosticToken(&'static str);

impl StaticDiagnosticToken {
    /// Creates a token when `value` is a 1..=64 byte ASCII token.
    pub const fn new(value: &'static str) -> Option<Self> {
        if is_valid_token(value, 64) {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// One value in the closed diagnostic attribute value set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticFieldValue {
    StaticToken(StaticDiagnosticToken),
    Bool(bool),
    I64(i64),
    U64(u64),
}

impl From<StaticDiagnosticToken> for DiagnosticFieldValue {
    fn from(value: StaticDiagnosticToken) -> Self {
        Self::StaticToken(value)
    }
}

impl From<bool> for DiagnosticFieldValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for DiagnosticFieldValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<u64> for DiagnosticFieldValue {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

/// The closed result of attempting to record one diagnostic attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum DiagnosticAttributeRecordOutcome {
    Recorded,
    Duplicate,
    Truncated,
}

/// A bounded, insertion-ordered collection of private diagnostic attributes.
///
/// Duplicate keys preserve the first value. Once the capacity is exhausted,
/// later unique fields are ignored and the collection records truncation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticAttributes {
    fields: Vec<(DiagnosticFieldKey, DiagnosticFieldValue)>,
    truncated: bool,
}

impl DiagnosticAttributes {
    pub const fn new() -> Self {
        Self {
            fields: Vec::new(),
            truncated: false,
        }
    }

    pub fn record(
        &mut self,
        key: DiagnosticFieldKey,
        value: DiagnosticFieldValue,
    ) -> DiagnosticAttributeRecordOutcome {
        if self.fields.iter().any(|(existing, _)| *existing == key) {
            return DiagnosticAttributeRecordOutcome::Duplicate;
        }
        if self.fields.len() == MAX_DIAGNOSTIC_ATTRIBUTES {
            self.truncated = true;
            return DiagnosticAttributeRecordOutcome::Truncated;
        }
        self.fields.push((key, value));
        DiagnosticAttributeRecordOutcome::Recorded
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DiagnosticFieldKey, &DiagnosticFieldValue)> {
        self.fields.iter().map(|(key, value)| (key, value))
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn was_truncated(&self) -> bool {
        self.truncated
    }
}

impl Default for DiagnosticAttributes {
    fn default() -> Self {
        Self::new()
    }
}

/// Common diagnostic capability for ordinary runtime errors.
pub trait RuntimeDiagnostic: Error + Send + Sync + 'static {
    fn diagnostic_code(&self) -> DiagnosticCode;

    fn diagnostic_message(&self) -> Cow<'_, str>;

    fn record_diagnostic_attributes(&self, _attributes: &mut DiagnosticAttributes) {}
}

const fn is_valid_token(value: &str, maximum_len: usize) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > maximum_len {
        return false;
    }

    let mut index = 0;
    while index < bytes.len() {
        if !matches!(
            bytes[index],
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'
        ) {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests;
