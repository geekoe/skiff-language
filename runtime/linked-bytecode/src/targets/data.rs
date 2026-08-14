use std::fmt;

use skiff_artifact_model::{
    LiteralIr, PackageBuildId, PackageSymbolRef, PrivilegedAffineCompositeIdentity, TypeRefIr,
};

use crate::{
    ArtifactConstantIndex, ArtifactConstantNodeIndex, ArtifactFunctionKey, ArtifactShapeIndex,
    ArtifactTypeIndex, ConstantIndex, FrozenConstantNodeIndex, FunctionIndex,
    LinkedValueTransferPlan, ShapeIndex, SpecializationKey, TypeIndex,
};

/// Exact origin of one concrete row produced from an artifact pool entry.
/// `specialization == None` means the row was package-global and required no
/// function-local substitution; otherwise the complete specialization key is
/// retained so one artifact row may legitimately produce multiple rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedArtifactPoolOrigin<I> {
    package_build_id: PackageBuildId,
    artifact_index: I,
    specialization: Option<SpecializationKey>,
}

impl<I> LinkedArtifactPoolOrigin<I> {
    pub fn new(
        package_build_id: PackageBuildId,
        artifact_index: I,
        specialization: Option<SpecializationKey>,
    ) -> Result<Self, LinkedArtifactPoolOriginError> {
        if let Some(specialization) = &specialization {
            if specialization.package_build_id() != &package_build_id {
                return Err(LinkedArtifactPoolOriginError::SpecializationOwnerMismatch {
                    pool_owner: package_build_id,
                    specialization_owner: specialization.package_build_id().clone(),
                });
            }
        }
        Ok(Self {
            package_build_id,
            artifact_index,
            specialization,
        })
    }

    pub const fn package_build_id(&self) -> &PackageBuildId {
        &self.package_build_id
    }

    pub const fn artifact_index(&self) -> &I {
        &self.artifact_index
    }

    pub const fn specialization(&self) -> Option<&SpecializationKey> {
        self.specialization.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedArtifactPoolOriginError {
    SpecializationOwnerMismatch {
        pool_owner: PackageBuildId,
        specialization_owner: PackageBuildId,
    },
}

impl fmt::Display for LinkedArtifactPoolOriginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpecializationOwnerMismatch {
                pool_owner,
                specialization_owner,
            } => write!(
                formatter,
                "artifact pool owner {pool_owner} does not match specialization owner {specialization_owner}"
            ),
        }
    }
}

impl std::error::Error for LinkedArtifactPoolOriginError {}

/// One concrete child position in a linked container layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedContainerPosition {
    ty: TypeIndex,
    plan: LinkedValueTransferPlan,
}

impl LinkedContainerPosition {
    pub fn new(ty: TypeIndex, plan: LinkedValueTransferPlan) -> Self {
        Self { ty, plan }
    }

    pub const fn ty(&self) -> TypeIndex {
        self.ty
    }

    pub const fn plan(&self) -> &LinkedValueTransferPlan {
        &self.plan
    }
}

/// Exact position lifecycle facts for the four built-in recursive containers.
/// Every position is already concrete; artifact `FromType` expressions have
/// no representation here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedContainerLayout {
    data: LinkedContainerLayoutData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LinkedContainerLayoutData {
    Array {
        element: LinkedContainerPosition,
    },
    Map {
        key: LinkedContainerPosition,
        value: LinkedContainerPosition,
    },
    Json {
        recursive_value: LinkedContainerPosition,
    },
    JsonObject {
        key: LinkedContainerPosition,
        value: LinkedContainerPosition,
    },
}

impl LinkedContainerLayout {
    pub fn array(element: LinkedContainerPosition) -> Self {
        Self {
            data: LinkedContainerLayoutData::Array { element },
        }
    }

    pub fn map(key: LinkedContainerPosition, value: LinkedContainerPosition) -> Self {
        Self {
            data: LinkedContainerLayoutData::Map { key, value },
        }
    }

    pub fn json(recursive_value: LinkedContainerPosition) -> Self {
        Self {
            data: LinkedContainerLayoutData::Json { recursive_value },
        }
    }

    pub fn json_object(key: LinkedContainerPosition, value: LinkedContainerPosition) -> Self {
        Self {
            data: LinkedContainerLayoutData::JsonObject { key, value },
        }
    }

    pub const fn kind(&self) -> LinkedContainerLayoutKind {
        match &self.data {
            LinkedContainerLayoutData::Array { .. } => LinkedContainerLayoutKind::Array,
            LinkedContainerLayoutData::Map { .. } => LinkedContainerLayoutKind::Map,
            LinkedContainerLayoutData::Json { .. } => LinkedContainerLayoutKind::Json,
            LinkedContainerLayoutData::JsonObject { .. } => LinkedContainerLayoutKind::JsonObject,
        }
    }

    pub const fn element(&self) -> Option<&LinkedContainerPosition> {
        match &self.data {
            LinkedContainerLayoutData::Array { element } => Some(element),
            _ => None,
        }
    }

    pub const fn key(&self) -> Option<&LinkedContainerPosition> {
        match &self.data {
            LinkedContainerLayoutData::Map { key, .. }
            | LinkedContainerLayoutData::JsonObject { key, .. } => Some(key),
            _ => None,
        }
    }

    pub const fn value(&self) -> Option<&LinkedContainerPosition> {
        match &self.data {
            LinkedContainerLayoutData::Map { value, .. }
            | LinkedContainerLayoutData::JsonObject { value, .. } => Some(value),
            _ => None,
        }
    }

    pub const fn recursive_value(&self) -> Option<&LinkedContainerPosition> {
        match &self.data {
            LinkedContainerLayoutData::Json { recursive_value } => Some(recursive_value),
            _ => None,
        }
    }

    pub fn positions(&self) -> impl Iterator<Item = &LinkedContainerPosition> {
        let positions = match &self.data {
            LinkedContainerLayoutData::Array { element } => [Some(element), None],
            LinkedContainerLayoutData::Map { key, value }
            | LinkedContainerLayoutData::JsonObject { key, value } => [Some(key), Some(value)],
            LinkedContainerLayoutData::Json { recursive_value } => [Some(recursive_value), None],
        };
        positions.into_iter().flatten()
    }

    pub fn position_entries(
        &self,
    ) -> impl Iterator<Item = (LinkedContainerPositionKind, &LinkedContainerPosition)> {
        let positions = match &self.data {
            LinkedContainerLayoutData::Array { element } => [
                Some((LinkedContainerPositionKind::ArrayElement, element)),
                None,
            ],
            LinkedContainerLayoutData::Map { key, value } => [
                Some((LinkedContainerPositionKind::MapKey, key)),
                Some((LinkedContainerPositionKind::MapValue, value)),
            ],
            LinkedContainerLayoutData::Json { recursive_value } => [
                Some((
                    LinkedContainerPositionKind::JsonRecursiveValue,
                    recursive_value,
                )),
                None,
            ],
            LinkedContainerLayoutData::JsonObject { key, value } => [
                Some((LinkedContainerPositionKind::JsonObjectKey, key)),
                Some((LinkedContainerPositionKind::JsonObjectValue, value)),
            ],
        };
        positions.into_iter().flatten()
    }
}

/// Closed container discriminator used for local shape diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedContainerLayoutKind {
    Array,
    Map,
    Json,
    JsonObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedContainerPositionKind {
    ArrayElement,
    MapKey,
    MapValue,
    JsonRecursiveValue,
    JsonObjectKey,
    JsonObjectValue,
}

/// Candidate type entry with exact artifact provenance and, for built-in
/// containers, an exact concrete position layout. The verifier must still
/// reject any residual `TypeParam` in the retained `TypeRefIr` and rederive
/// every layout from the pinned lifecycle registry.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedTypeEntry {
    index: TypeIndex,
    origin: LinkedArtifactPoolOrigin<ArtifactTypeIndex>,
    type_ref: TypeRefIr,
    container_layout: Option<LinkedContainerLayout>,
}

impl LinkedTypeEntry {
    pub fn new(
        index: TypeIndex,
        origin: LinkedArtifactPoolOrigin<ArtifactTypeIndex>,
        type_ref: TypeRefIr,
        container_layout: Option<LinkedContainerLayout>,
    ) -> Self {
        Self {
            index,
            origin,
            type_ref,
            container_layout,
        }
    }

    pub const fn index(&self) -> TypeIndex {
        self.index
    }

    pub const fn origin(&self) -> &LinkedArtifactPoolOrigin<ArtifactTypeIndex> {
        &self.origin
    }

    pub const fn type_ref(&self) -> &TypeRefIr {
        &self.type_ref
    }

    pub const fn container_layout(&self) -> Option<&LinkedContainerLayout> {
        self.container_layout.as_ref()
    }
}

/// One nominal shape field in exact dense ordinal order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedShapeField {
    name: Box<str>,
    ty: TypeIndex,
    plan: LinkedValueTransferPlan,
}

impl LinkedShapeField {
    pub fn new(
        name: impl Into<String>,
        ty: TypeIndex,
        plan: LinkedValueTransferPlan,
    ) -> Result<Self, LinkedShapeError> {
        let name = name.into();
        if name.is_empty() {
            return Err(LinkedShapeError::EmptyFieldName);
        }
        Ok(Self {
            name: name.into_boxed_str(),
            ty,
            plan,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn ty(&self) -> TypeIndex {
        self.ty
    }

    pub const fn plan(&self) -> &LinkedValueTransferPlan {
        &self.plan
    }
}

/// Linked nominal shape. Equal field layouts with different nominal types or
/// artifact owners remain distinct rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedShapeEntry {
    index: ShapeIndex,
    origin: LinkedArtifactPoolOrigin<ArtifactShapeIndex>,
    nominal_type: TypeIndex,
    privileged_affine_composite: Option<PrivilegedAffineCompositeIdentity>,
    fields: Box<[LinkedShapeField]>,
}

impl LinkedShapeEntry {
    pub fn new(
        index: ShapeIndex,
        origin: LinkedArtifactPoolOrigin<ArtifactShapeIndex>,
        nominal_type: TypeIndex,
        privileged_affine_composite: Option<PrivilegedAffineCompositeIdentity>,
        fields: Box<[LinkedShapeField]>,
    ) -> Result<Self, LinkedShapeError> {
        let mut previous_name: Option<&str> = None;
        for field in &fields {
            if let Some(previous_name) = previous_name {
                if previous_name >= field.name() {
                    return Err(LinkedShapeError::NonCanonicalFieldOrder {
                        previous: previous_name.to_string(),
                        current: field.name().to_string(),
                    });
                }
            }
            previous_name = Some(field.name());
        }
        Ok(Self {
            index,
            origin,
            nominal_type,
            privileged_affine_composite,
            fields,
        })
    }

    pub const fn index(&self) -> ShapeIndex {
        self.index
    }

    pub const fn origin(&self) -> &LinkedArtifactPoolOrigin<ArtifactShapeIndex> {
        &self.origin
    }

    pub const fn nominal_type(&self) -> TypeIndex {
        self.nominal_type
    }

    /// Registry-owned authority transported from the exact admitted artifact
    /// shape. `None` is an ordinary shape and is never upgraded by matching a
    /// nominal name or field layout.
    pub const fn privileged_affine_composite(&self) -> Option<PrivilegedAffineCompositeIdentity> {
        self.privileged_affine_composite
    }

    pub fn fields(&self) -> &[LinkedShapeField] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedShapeError {
    EmptyFieldName,
    NonCanonicalFieldOrder { previous: String, current: String },
}

impl fmt::Display for LinkedShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFieldName => {
                formatter.write_str("linked shape field name must not be empty")
            }
            Self::NonCanonicalFieldOrder { previous, current } => write!(
                formatter,
                "linked shape field {current:?} must sort after {previous:?}"
            ),
        }
    }
}

impl std::error::Error for LinkedShapeError {}

/// Owner-aware origin of one linked constant-pool row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedConstantReference {
    LocalNode {
        node: FrozenConstantNodeIndex,
    },
    PackageSymbol {
        source: PackageSymbolRef,
        resolved_origin: LinkedArtifactPoolOrigin<ArtifactConstantNodeIndex>,
        node: FrozenConstantNodeIndex,
    },
}

impl LinkedConstantReference {
    pub const fn node(&self) -> FrozenConstantNodeIndex {
        match self {
            Self::LocalNode { node } | Self::PackageSymbol { node, .. } => *node,
        }
    }
}

/// Constant-pool row used by `const`: exact source, output type and complete
/// lifecycle plan. Package-symbol rows cannot exist without an exact resolved
/// build and artifact graph node.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedConstantEntry {
    index: ConstantIndex,
    origin: LinkedArtifactPoolOrigin<ArtifactConstantIndex>,
    reference: LinkedConstantReference,
    ty: TypeIndex,
    plan: LinkedValueTransferPlan,
}

impl LinkedConstantEntry {
    pub fn new(
        index: ConstantIndex,
        origin: LinkedArtifactPoolOrigin<ArtifactConstantIndex>,
        reference: LinkedConstantReference,
        ty: TypeIndex,
        plan: LinkedValueTransferPlan,
    ) -> Self {
        Self {
            index,
            origin,
            reference,
            ty,
            plan,
        }
    }

    pub const fn index(&self) -> ConstantIndex {
        self.index
    }

    pub const fn origin(&self) -> &LinkedArtifactPoolOrigin<ArtifactConstantIndex> {
        &self.origin
    }

    pub const fn reference(&self) -> &LinkedConstantReference {
        &self.reference
    }

    pub const fn ty(&self) -> TypeIndex {
        self.ty
    }

    pub const fn plan(&self) -> &LinkedValueTransferPlan {
        &self.plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedConstantSymbolPath(Box<str>);

impl LinkedConstantSymbolPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, LinkedConstantSymbolPathParseError> {
        let value = value.into();
        if value.is_empty() {
            return Err(LinkedConstantSymbolPathParseError::Empty);
        }
        if let Some((character_index, _)) = value
            .chars()
            .enumerate()
            .find(|(_, character)| character.is_control())
        {
            return Err(LinkedConstantSymbolPathParseError::ControlCharacter {
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
pub enum LinkedConstantSymbolPathParseError {
    Empty,
    ControlCharacter {
        value: String,
        character_index: usize,
    },
}

impl fmt::Display for LinkedConstantSymbolPathParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("constant symbol path must not be empty"),
            Self::ControlCharacter {
                value,
                character_index,
            } => write!(
                formatter,
                "constant symbol path {value:?} contains a control character at character index {character_index}"
            ),
        }
    }
}

impl std::error::Error for LinkedConstantSymbolPathParseError {}

/// Exact artifact constant-root mapping retained for root discovery and
/// independent hydration comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedConstantRoot {
    owner_package_build_id: PackageBuildId,
    symbol_path: LinkedConstantSymbolPath,
    constant: ConstantIndex,
}

impl LinkedConstantRoot {
    pub fn new(
        owner_package_build_id: PackageBuildId,
        symbol_path: LinkedConstantSymbolPath,
        constant: ConstantIndex,
    ) -> Self {
        Self {
            owner_package_build_id,
            symbol_path,
            constant,
        }
    }

    pub const fn owner_package_build_id(&self) -> &PackageBuildId {
        &self.owner_package_build_id
    }

    pub const fn symbol_path(&self) -> &LinkedConstantSymbolPath {
        &self.symbol_path
    }

    pub const fn constant(&self) -> ConstantIndex {
        self.constant
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFrozenBehaviorBinding {
    artifact_function_key: ArtifactFunctionKey,
    function: FunctionIndex,
}

impl LinkedFrozenBehaviorBinding {
    pub fn new(artifact_function_key: ArtifactFunctionKey, function: FunctionIndex) -> Self {
        Self {
            artifact_function_key,
            function,
        }
    }

    pub const fn artifact_function_key(&self) -> &ArtifactFunctionKey {
        &self.artifact_function_key
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }
}

/// Relational linked frozen graph node value.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkedFrozenConstantValue {
    Literal(LiteralIr),
    Array {
        children: Box<[FrozenConstantNodeIndex]>,
    },
    Record {
        shape: ShapeIndex,
        children: Box<[FrozenConstantNodeIndex]>,
    },
    Representation {
        ty: TypeIndex,
        value: FrozenConstantNodeIndex,
    },
    Implementation {
        record: FrozenConstantNodeIndex,
        behaviors: Box<[LinkedFrozenBehaviorBinding]>,
    },
}

impl LinkedFrozenConstantValue {
    pub fn children(&self) -> &[FrozenConstantNodeIndex] {
        match self {
            Self::Array { children } | Self::Record { children, .. } => children,
            Self::Representation { value, .. } => std::slice::from_ref(value),
            Self::Implementation { record, .. } => std::slice::from_ref(record),
            Self::Literal(_) => &[],
        }
    }
}

/// Exact owner/artifact row in the global linked frozen graph.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedFrozenConstantNode {
    index: FrozenConstantNodeIndex,
    origin: LinkedArtifactPoolOrigin<ArtifactConstantNodeIndex>,
    value: LinkedFrozenConstantValue,
}

impl LinkedFrozenConstantNode {
    pub fn new(
        index: FrozenConstantNodeIndex,
        origin: LinkedArtifactPoolOrigin<ArtifactConstantNodeIndex>,
        value: LinkedFrozenConstantValue,
    ) -> Self {
        Self {
            index,
            origin,
            value,
        }
    }

    pub const fn index(&self) -> FrozenConstantNodeIndex {
        self.index
    }

    pub const fn origin(&self) -> &LinkedArtifactPoolOrigin<ArtifactConstantNodeIndex> {
        &self.origin
    }

    pub const fn value(&self) -> &LinkedFrozenConstantValue {
        &self.value
    }
}
