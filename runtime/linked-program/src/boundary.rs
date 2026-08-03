use std::{collections::HashMap, sync::Arc};

use super::{
    ConstAddr, ExecutableAddr, LinkOverlay, LinkedFileUnit, PublicationResourceTable,
    RuntimeExecutionPackage, RuntimeProgramResourceLookupError, RuntimeTypeContext, UnitAddr,
};
use skiff_runtime_model::resource::LoadedPublicationResource;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeProgramIdentity {
    pub dynamic_build_id: String,
    pub linked_image_identity: String,
}

impl RuntimeProgramIdentity {
    pub fn from_dynamic_build_id(dynamic_build_id: impl Into<String>) -> Self {
        let dynamic_build_id = dynamic_build_id.into();
        Self {
            linked_image_identity: dynamic_build_id.clone(),
            dynamic_build_id,
        }
    }

    pub fn new(
        dynamic_build_id: impl Into<String>,
        linked_image_identity: impl Into<String>,
    ) -> Self {
        Self {
            dynamic_build_id: dynamic_build_id.into(),
            linked_image_identity: linked_image_identity.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LinkedProgramImage {
    pub service_files: Vec<Arc<LinkedFileUnit>>,
    pub packages: Vec<Arc<RuntimeExecutionPackage>>,
    pub service_resources: PublicationResourceTable,
    pub routes: HashMap<String, ExecutableAddr>,
    pub task_routes: HashMap<String, ExecutableAddr>,
    pub operations: HashMap<String, ExecutableAddr>,
    pub operation_receivers: HashMap<String, ConstAddr>,
    pub link_overlay: LinkOverlay,
    pub types: RuntimeTypeContext,
}

impl LinkedProgramImage {
    pub fn resource_view(&self) -> RuntimeExecutionResourceView<'_> {
        RuntimeExecutionResourceView::new(&self.service_resources, &self.packages)
    }
}

/// Resource lookup over the same exact package contexts used for code and type
/// resolution.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeExecutionResourceView<'a> {
    service_resources: &'a PublicationResourceTable,
    packages: &'a [Arc<RuntimeExecutionPackage>],
}

impl<'a> RuntimeExecutionResourceView<'a> {
    pub fn new(
        service_resources: &'a PublicationResourceTable,
        packages: &'a [Arc<RuntimeExecutionPackage>],
    ) -> Self {
        Self {
            service_resources,
            packages,
        }
    }

    pub fn lookup(
        self,
        owner: &UnitAddr,
        path: &str,
    ) -> Result<Option<&'a LoadedPublicationResource>, RuntimeProgramResourceLookupError> {
        match owner {
            UnitAddr::Service => Ok(self.service_resources.get(path)),
            UnitAddr::Package(slot) => self
                .packages
                .get(*slot)
                .map(|package| package.static_resources().get(path))
                .ok_or(RuntimeProgramResourceLookupError::PackageSlotOutOfBounds {
                    slot: *slot,
                    package_count: self.packages.len(),
                }),
        }
    }
}
