use std::{collections::HashMap, sync::Arc};

use super::{
    ConstAddr, ExecutableAddr, LinkOverlay, LinkedFileUnit, PackageUnit, PublicationResourceTable,
    RuntimeProgramResourceView, RuntimeTypeContext,
};

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
    pub packages: Vec<Arc<PackageUnit>>,
    pub package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
    pub service_resources: PublicationResourceTable,
    pub package_resources: Vec<PublicationResourceTable>,
    pub routes: HashMap<String, ExecutableAddr>,
    pub spawn_routes: HashMap<String, ExecutableAddr>,
    pub operations: HashMap<String, ExecutableAddr>,
    pub operation_receivers: HashMap<String, ConstAddr>,
    pub link_overlay: LinkOverlay,
    pub types: RuntimeTypeContext,
}

impl LinkedProgramImage {
    pub fn resource_view(&self) -> RuntimeProgramResourceView<'_> {
        RuntimeProgramResourceView::new(&self.service_resources, &self.package_resources)
    }
}
