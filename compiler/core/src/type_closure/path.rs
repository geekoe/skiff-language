use skiff_artifact_model::{LiteralIr, TypeRefIr};

use crate::type_ref::TypeRefVisitPathSegment;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeClosureTraceSegment {
    NativeArg { name: String, index: usize },
    RecordField { name: String },
    UnionItem { index: usize },
    NullableInner,
    AnyInterfaceTypeArg { index: usize },
    FunctionParam { name: String, index: usize },
    FunctionReturn,
    Nominal { module_path: String, name: String },
    AliasTarget,
    DeclarationField { name: String },
    DeclarationVariant { index: usize },
}

impl From<TypeRefVisitPathSegment> for TypeClosureTraceSegment {
    fn from(segment: TypeRefVisitPathSegment) -> Self {
        match segment {
            TypeRefVisitPathSegment::NativeArg { name, index } => Self::NativeArg { name, index },
            TypeRefVisitPathSegment::RecordField { name } => Self::RecordField { name },
            TypeRefVisitPathSegment::UnionItem { index } => Self::UnionItem { index },
            TypeRefVisitPathSegment::NullableInner => Self::NullableInner,
            TypeRefVisitPathSegment::AnyInterfaceTypeArg { index } => {
                Self::AnyInterfaceTypeArg { index }
            }
            TypeRefVisitPathSegment::FunctionParam { name, index } => {
                Self::FunctionParam { name, index }
            }
            TypeRefVisitPathSegment::FunctionReturn => Self::FunctionReturn,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeClosureTrace {
    segments: Vec<TypeClosureTraceSegment>,
}

impl TypeClosureTrace {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn segments(&self) -> &[TypeClosureTraceSegment] {
        &self.segments
    }

    pub fn child(&self, segment: TypeClosureTraceSegment) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment);
        Self { segments }
    }
}

pub trait TypeClosureGuardPolicy {
    fn child_is_guarded(
        &self,
        parent: &TypeRefIr,
        segment: &TypeClosureTraceSegment,
        inherited: bool,
    ) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoTypeClosureGuards;

impl TypeClosureGuardPolicy for NoTypeClosureGuards {
    fn child_is_guarded(
        &self,
        _parent: &TypeRefIr,
        _segment: &TypeClosureTraceSegment,
        inherited: bool,
    ) -> bool {
        inherited
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RepresentationIndirectionGuards;

impl TypeClosureGuardPolicy for RepresentationIndirectionGuards {
    fn child_is_guarded(
        &self,
        parent: &TypeRefIr,
        segment: &TypeClosureTraceSegment,
        inherited: bool,
    ) -> bool {
        inherited
            || matches!(segment, TypeClosureTraceSegment::NullableInner)
            || matches!(parent, TypeRefIr::Builtin { name, .. } if matches!(name.as_str(), "Array" | "Map"))
            || matches!(
                (parent, segment),
                (
                    TypeRefIr::Union { items },
                    TypeClosureTraceSegment::UnionItem { index }
                ) if !is_null(&items[*index]) && items.iter().any(is_null)
            )
    }
}

fn is_null(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "null" && args.is_empty()
    ) || matches!(
        ty,
        TypeRefIr::Literal {
            value: LiteralIr::Null
        }
    )
}
