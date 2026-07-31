use std::collections::BTreeSet;

use skiff_artifact_model::TypeRefIr;

use crate::type_ref::{walk_type_ref_with_path, TypeRefVisitPath};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeGraphNodeKind {
    Native { name: String },
    LocalType,
    PublicationType,
    ServiceSymbol,
    PackageSymbol,
    AppliedNominal,
    DbObjectSymbol,
    Record,
    Union,
    Nullable,
    Literal,
    TypeParam,
    AnyInterface,
    Function,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeGraphNodeFact {
    pub path: TypeRefVisitPath,
    pub kind: TypeGraphNodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeGraphFacts {
    pub contains_function: bool,
    pub contains_any_interface: bool,
    pub contains_native: bool,
    pub contains_local_type: bool,
    pub contains_package_symbol: bool,
    pub contains_service_symbol: bool,
    pub contains_db_object_symbol: bool,
    pub contains_type_param: bool,
    pub schema_projectable_plain_data: bool,
}

impl Default for TypeGraphFacts {
    fn default() -> Self {
        Self {
            contains_function: false,
            contains_any_interface: false,
            contains_native: false,
            contains_local_type: false,
            contains_package_symbol: false,
            contains_service_symbol: false,
            contains_db_object_symbol: false,
            contains_type_param: false,
            schema_projectable_plain_data: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeGraphAnalysis {
    pub nodes: Vec<TypeGraphNodeFact>,
    pub facts: TypeGraphFacts,
}

#[derive(Clone, Debug)]
pub struct TypeGraphAnalyzer {
    schema_projectable_native_names: BTreeSet<String>,
}

impl Default for TypeGraphAnalyzer {
    fn default() -> Self {
        Self {
            schema_projectable_native_names: [
                "Json", "Array", "Map", "bool", "boolean", "float", "int", "null", "number",
                "string", "unit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

impl TypeGraphAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn analyze(&self, ty: &TypeRefIr) -> TypeGraphAnalysis {
        let mut analysis = TypeGraphAnalysis::default();
        walk_type_ref_with_path(ty, &mut |visit| {
            let kind = self.node_kind(visit.ty);
            self.apply_facts(&mut analysis.facts, visit.ty);
            analysis.nodes.push(TypeGraphNodeFact {
                path: visit.path,
                kind,
            });
        });
        analysis
    }

    fn node_kind(&self, ty: &TypeRefIr) -> TypeGraphNodeKind {
        match ty {
            TypeRefIr::Builtin { name, .. } => TypeGraphNodeKind::Native { name: name.clone() },
            TypeRefIr::LocalType { .. } => TypeGraphNodeKind::LocalType,
            TypeRefIr::PublicationType { .. } => TypeGraphNodeKind::PublicationType,
            TypeRefIr::ServiceSymbol { .. } => TypeGraphNodeKind::ServiceSymbol,
            TypeRefIr::PackageSymbol { .. } => TypeGraphNodeKind::PackageSymbol,
            TypeRefIr::PackageSchema { .. } => TypeGraphNodeKind::PackageSymbol,
            TypeRefIr::AppliedNominal { .. } => TypeGraphNodeKind::AppliedNominal,
            TypeRefIr::DbObjectSymbol { .. } => TypeGraphNodeKind::DbObjectSymbol,
            TypeRefIr::Record { .. } => TypeGraphNodeKind::Record,
            TypeRefIr::Union { .. } => TypeGraphNodeKind::Union,
            TypeRefIr::Nullable { .. } => TypeGraphNodeKind::Nullable,
            TypeRefIr::Literal { .. } => TypeGraphNodeKind::Literal,
            TypeRefIr::TypeParam { .. } => TypeGraphNodeKind::TypeParam,
            TypeRefIr::AnyInterface { .. } => TypeGraphNodeKind::AnyInterface,
            TypeRefIr::Function { .. } => TypeGraphNodeKind::Function,
        }
    }

    fn apply_facts(&self, facts: &mut TypeGraphFacts, ty: &TypeRefIr) {
        match ty {
            TypeRefIr::Builtin { name, .. } => {
                facts.contains_native = true;
                if !self.schema_projectable_native_names.contains(name) {
                    facts.schema_projectable_plain_data = false;
                }
            }
            TypeRefIr::LocalType { .. } => {
                facts.contains_local_type = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::PublicationType { .. } => {
                facts.contains_local_type = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::ServiceSymbol { .. } => {
                facts.contains_service_symbol = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::PackageSymbol { .. } | TypeRefIr::PackageSchema { .. } => {
                facts.contains_package_symbol = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::AppliedNominal { base, .. } => {
                use skiff_artifact_model::NominalTypeRefBaseIr;

                match base {
                    NominalTypeRefBaseIr::LocalType { .. }
                    | NominalTypeRefBaseIr::PublicationType { .. } => {
                        facts.contains_local_type = true;
                    }
                    NominalTypeRefBaseIr::ServiceSymbol { .. } => {
                        facts.contains_service_symbol = true;
                    }
                    NominalTypeRefBaseIr::PackageSymbol { .. }
                    | NominalTypeRefBaseIr::PackageSchema { .. } => {
                        facts.contains_package_symbol = true;
                    }
                }
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::DbObjectSymbol { .. } => {
                facts.contains_db_object_symbol = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::TypeParam { .. } => {
                facts.contains_type_param = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::AnyInterface { .. } => {
                facts.contains_any_interface = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::Function { .. } => {
                facts.contains_function = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests;
