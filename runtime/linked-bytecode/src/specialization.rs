use std::fmt;

use skiff_artifact_model::{PackageBuildId, PackageCallableId};

use crate::TypeIndex;

/// Exact key of a template function in one hydrated bytecode artifact.
///
/// Artifact model currently represents this map key as a string. This local
/// newtype prevents it from being confused with the callable's semantic
/// [`PackageCallableId`]. Because artifact model defines no richer key parser,
/// this type imposes only a minimal fail-closed lexical guard: non-empty, with
/// no whitespace or control characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactFunctionKey(Box<str>);

impl ArtifactFunctionKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, ArtifactFunctionKeyParseError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ArtifactFunctionKeyParseError::Empty);
        }
        if let Some((character_index, _)) = value
            .chars()
            .enumerate()
            .find(|(_, character)| character.is_whitespace() || character.is_control())
        {
            return Err(ArtifactFunctionKeyParseError::WhitespaceOrControl {
                value,
                character_index,
            });
        }
        Ok(Self(value.into_boxed_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactFunctionKeyParseError {
    Empty,
    WhitespaceOrControl {
        value: String,
        character_index: usize,
    },
}

impl fmt::Display for ArtifactFunctionKeyParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("artifact function key must not be empty"),
            Self::WhitespaceOrControl {
                value,
                character_index,
            } => write!(
                formatter,
                "artifact function key {value:?} contains whitespace or a control character at character index {character_index}"
            ),
        }
    }
}

impl std::error::Error for ArtifactFunctionKeyParseError {}

/// Canonical identity of one concrete deployment-link specialization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpecializationKey {
    package_build_id: PackageBuildId,
    artifact_function_key: ArtifactFunctionKey,
    template_function_key: PackageCallableId,
    concrete_type_arguments: Box<[TypeIndex]>,
    concrete_receiver: Option<TypeIndex>,
}

impl SpecializationKey {
    pub fn new(
        package_build_id: PackageBuildId,
        artifact_function_key: ArtifactFunctionKey,
        template_function_key: PackageCallableId,
        concrete_type_arguments: Box<[TypeIndex]>,
        concrete_receiver: Option<TypeIndex>,
    ) -> Self {
        Self {
            package_build_id,
            artifact_function_key,
            template_function_key,
            concrete_type_arguments,
            concrete_receiver,
        }
    }

    pub fn package_build_id(&self) -> &PackageBuildId {
        &self.package_build_id
    }

    pub fn artifact_function_key(&self) -> &ArtifactFunctionKey {
        &self.artifact_function_key
    }

    pub fn template_function_key(&self) -> &PackageCallableId {
        &self.template_function_key
    }

    pub fn concrete_type_arguments(&self) -> &[TypeIndex] {
        &self.concrete_type_arguments
    }

    pub const fn concrete_receiver(&self) -> Option<TypeIndex> {
        self.concrete_receiver
    }
}
