use std::fmt;

use skiff_artifact_model::{
    bytecode::dto::DbOperationKind, BuiltinReceiverOp, FileIrRef, PackageArtifactRef,
};

use crate::{
    IntrinsicIndex, LinkedNativeCallableSignature, LinkedTaskTarget, LinkedValueTransferPlan,
    TypeIndex,
};

/// Validated static intrinsic key. It is an untrusted registry claim, not an
/// authority token.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedIntrinsicCanonicalKey(Box<str>);

impl LinkedIntrinsicCanonicalKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, LinkedIntrinsicTargetError> {
        let value = value.into();
        if value.is_empty() {
            return Err(LinkedIntrinsicTargetError::EmptyCanonicalKey);
        }
        if let Some((character_index, _)) = value
            .chars()
            .enumerate()
            .find(|(_, character)| character.is_whitespace() || character.is_control())
        {
            return Err(LinkedIntrinsicTargetError::InvalidCanonicalKey {
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
pub struct LinkedStaticIntrinsicTarget {
    canonical_key: LinkedIntrinsicCanonicalKey,
    signature_version: u32,
}

impl LinkedStaticIntrinsicTarget {
    pub fn new(
        canonical_key: LinkedIntrinsicCanonicalKey,
        signature_version: u32,
    ) -> Result<Self, LinkedIntrinsicTargetError> {
        if signature_version == 0 {
            return Err(LinkedIntrinsicTargetError::ZeroSignatureVersion);
        }
        Ok(Self {
            canonical_key,
            signature_version,
        })
    }

    pub const fn canonical_key(&self) -> &LinkedIntrinsicCanonicalKey {
        &self.canonical_key
    }

    pub const fn signature_version(&self) -> u32 {
        self.signature_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedIntrinsicKind {
    Static(LinkedStaticIntrinsicTarget),
    Receiver(BuiltinReceiverOp),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedIntrinsicTarget {
    index: IntrinsicIndex,
    kind: LinkedIntrinsicKind,
    signature: LinkedNativeCallableSignature,
    db_operation: Option<LinkedDbOperation>,
    task_target: Option<LinkedTaskTarget>,
}

impl LinkedIntrinsicTarget {
    pub fn new(
        index: IntrinsicIndex,
        kind: LinkedIntrinsicKind,
        signature: LinkedNativeCallableSignature,
    ) -> Self {
        Self {
            index,
            kind,
            signature,
            db_operation: None,
            task_target: None,
        }
    }

    pub const fn index(&self) -> IntrinsicIndex {
        self.index
    }

    pub const fn kind(&self) -> &LinkedIntrinsicKind {
        &self.kind
    }

    pub const fn signature(&self) -> &LinkedNativeCallableSignature {
        &self.signature
    }

    pub fn with_db_operation(mut self, db_operation: LinkedDbOperation) -> Self {
        self.db_operation = Some(db_operation);
        self
    }

    pub const fn db_operation(&self) -> Option<&LinkedDbOperation> {
        self.db_operation.as_ref()
    }

    pub fn with_task_target(mut self, task_target: LinkedTaskTarget) -> Self {
        self.task_target = Some(task_target);
        self
    }

    pub const fn task_target(&self) -> Option<&LinkedTaskTarget> {
        self.task_target.as_ref()
    }
}

/// Exact linked DB object target identity.
///
/// This is the image-local form of
/// `DbObjectTargetId(PackageArtifactRef, FileIrRef, typeIndex)`. Runtime
/// consumers must use this identity and never reconstruct a target from
/// `type_name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedDbObjectTargetId {
    package_artifact_ref: PackageArtifactRef,
    file_ir_ref: FileIrRef,
    type_index: u32,
}

impl LinkedDbObjectTargetId {
    pub fn new(
        package_artifact_ref: PackageArtifactRef,
        file_ir_ref: FileIrRef,
        type_index: u32,
    ) -> Self {
        Self {
            package_artifact_ref,
            file_ir_ref,
            type_index,
        }
    }

    pub const fn package_artifact_ref(&self) -> &PackageArtifactRef {
        &self.package_artifact_ref
    }

    pub const fn file_ir_ref(&self) -> &FileIrRef {
        &self.file_ir_ref
    }

    pub const fn type_index(&self) -> u32 {
        self.type_index
    }
}

/// Exact compiler facts for one linked DB operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedDbOperation {
    target_id: LinkedDbObjectTargetId,
    type_name: Box<str>,
    op: DbOperationKind,
    parameter_plan: LinkedValueTransferPlan,
    result_type: TypeIndex,
    result_plan: LinkedValueTransferPlan,
}

impl LinkedDbOperation {
    pub fn new(
        target_id: LinkedDbObjectTargetId,
        type_name: impl Into<String>,
        op: DbOperationKind,
        parameter_plan: LinkedValueTransferPlan,
        result_type: TypeIndex,
        result_plan: LinkedValueTransferPlan,
    ) -> Result<Self, LinkedIntrinsicTargetError> {
        let type_name = type_name.into();
        if type_name.is_empty() {
            return Err(LinkedIntrinsicTargetError::EmptyTypeName);
        }
        Ok(Self {
            target_id,
            type_name: type_name.into_boxed_str(),
            op,
            parameter_plan,
            result_type,
            result_plan,
        })
    }

    pub const fn target_id(&self) -> &LinkedDbObjectTargetId {
        &self.target_id
    }

    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    pub const fn op(&self) -> DbOperationKind {
        self.op
    }

    pub const fn parameter_plan(&self) -> &LinkedValueTransferPlan {
        &self.parameter_plan
    }

    pub const fn result_type(&self) -> TypeIndex {
        self.result_type
    }

    pub const fn result_plan(&self) -> &LinkedValueTransferPlan {
        &self.result_plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedIntrinsicTargetError {
    EmptyCanonicalKey,
    EmptyTypeName,
    InvalidCanonicalKey {
        value: String,
        character_index: usize,
    },
    ZeroSignatureVersion,
}

impl fmt::Display for LinkedIntrinsicTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCanonicalKey => {
                formatter.write_str("intrinsic canonical key must not be empty")
            }
            Self::EmptyTypeName => formatter.write_str("db object type name must not be empty"),
            Self::InvalidCanonicalKey {
                value,
                character_index,
            } => write!(
                formatter,
                "intrinsic canonical key {value:?} contains whitespace or a control character at character index {character_index}"
            ),
            Self::ZeroSignatureVersion => {
                formatter.write_str("intrinsic signature version must be non-zero")
            }
        }
    }
}

impl std::error::Error for LinkedIntrinsicTargetError {}
