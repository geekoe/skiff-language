use std::collections::BTreeMap;

use skiff_artifact_model::PackageTypeRef;

use crate::{
    ExpressionKey, ResolvedTypeRef, SourceDependencyAnalysisInput, TypeResolutionContext,
    TypeResolutionModel,
};

use super::{project_source_package_type, projected_type_contains_contract};

#[derive(Clone, Debug, Default)]
pub(crate) struct ContractProjectionState {
    bindings: BTreeMap<String, PackageTypeRef>,
    expression_types: BTreeMap<ExpressionKey, PackageTypeRef>,
}

impl ContractProjectionState {
    pub(crate) fn new(
        env: &BTreeMap<String, ResolvedTypeRef>,
        type_resolution: &TypeResolutionModel,
        dependency_analysis: Option<&SourceDependencyAnalysisInput>,
        type_context: &TypeResolutionContext<'_>,
    ) -> Self {
        let bindings = dependency_analysis
            .map(|dependency_analysis| {
                env.iter()
                    .filter_map(|(name, ty)| {
                        let projected = project_source_package_type(
                            ty,
                            type_resolution,
                            dependency_analysis,
                            type_context,
                        );
                        projected_type_contains_contract(&projected)
                            .then(|| (name.clone(), projected))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            bindings,
            expression_types: BTreeMap::new(),
        }
    }

    pub(crate) fn binding_snapshot(&self) -> BTreeMap<String, PackageTypeRef> {
        self.bindings.clone()
    }

    pub(crate) fn restore_bindings(&mut self, bindings: BTreeMap<String, PackageTypeRef>) {
        self.bindings = bindings;
    }

    pub(crate) fn bind(&mut self, name: &str, projected: Option<PackageTypeRef>) {
        match projected {
            Some(projected) => {
                self.bindings.insert(name.to_string(), projected);
            }
            None => {
                self.bindings.remove(name);
            }
        }
    }

    pub(crate) fn expression_type(&self, key: &ExpressionKey) -> Option<&PackageTypeRef> {
        self.expression_types.get(key)
    }

    pub(crate) fn expression_types(&self) -> &BTreeMap<ExpressionKey, PackageTypeRef> {
        &self.expression_types
    }

    pub(crate) fn record_expression_type(&mut self, key: ExpressionKey, projected: PackageTypeRef) {
        self.expression_types.insert(key, projected);
    }

    pub(crate) fn inherit_identifier(&mut self, key: &ExpressionKey, name: &str) {
        if self.expression_types.contains_key(key) {
            return;
        }
        if let Some(projected) = self.bindings.get(name).cloned() {
            self.expression_types.insert(key.clone(), projected);
        }
    }
}
