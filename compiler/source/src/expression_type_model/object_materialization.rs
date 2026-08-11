use std::collections::{BTreeMap, BTreeSet};

use crate::{shared::error::SourceSpan, ExpressionKey, ResolvedTypeRef};

/// Source-owned lowering contract for one target-typed object literal.
///
/// The fact freezes both the resolved source target and the concrete runtime
/// materialization selected by source typing. Consumers must not inspect the
/// AST or re-run shape/union selection.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetTypedObjectMaterialization {
    pub resolved_target: ResolvedTypeRef,
    pub kind: ObjectMaterializationKind,
    pub fields: Vec<MaterializedObjectField>,
    pub source_fields: Vec<MaterializedObjectSourceField>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedObjectSourceField {
    pub name: String,
    pub ty: ResolvedTypeRef,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectMaterializationKind {
    Record { construct_target: ResolvedTypeRef },
    DiscriminatedUnionBranch { branch: ResolvedTypeRef },
    Map,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedObjectField {
    pub name: String,
    pub ty: ResolvedTypeRef,
    pub source: ObjectFieldValueSource,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectFieldValueSource {
    Provided { expression: ExpressionKey },
    SyntheticNull,
}

#[derive(Default)]
pub(super) struct ObjectMaterializationState {
    pub sources: BTreeMap<ExpressionKey, ObjectLiteralSource>,
    pub targeted: BTreeSet<ExpressionKey>,
    pub facts: BTreeMap<ExpressionKey, TargetTypedObjectMaterialization>,
}

#[derive(Clone, Debug)]
pub(super) struct ObjectLiteralSource {
    pub span: SourceSpan,
    pub fields: Vec<ObjectLiteralSourceField>,
    pub allow_targetless: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ObjectLiteralSourceField {
    pub name: String,
    pub expression: ExpressionKey,
    pub actual: Option<ResolvedTypeRef>,
    pub value_span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(super) struct ObjectMaterializationPlan {
    pub resolved_target: ResolvedTypeRef,
    pub kind: ObjectMaterializationKind,
    pub fields: BTreeMap<String, ResolvedTypeRef>,
}

#[cfg(test)]
mod tests;
