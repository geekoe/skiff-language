use skiff_artifact_model::PackageCallableId;

use crate::TypeIndex;

/// Canonical identity of one concrete deployment-link specialization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpecializationKey {
    template_function_key: PackageCallableId,
    concrete_type_arguments: Box<[TypeIndex]>,
    concrete_receiver: Option<TypeIndex>,
}

impl SpecializationKey {
    pub fn new(
        template_function_key: PackageCallableId,
        concrete_type_arguments: Box<[TypeIndex]>,
        concrete_receiver: Option<TypeIndex>,
    ) -> Self {
        Self {
            template_function_key,
            concrete_type_arguments,
            concrete_receiver,
        }
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
