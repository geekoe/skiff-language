use std::{collections::HashMap, sync::Arc};

use skiff_runtime_linked_program::{LinkOverlay, RuntimeExecutionPackage, RuntimeTypeContext};

use super::EvalRecoverableBehaviorHooks;
use crate::invocation::EvalProgramProjection;

const PACKAGE_ID: &str = "skiff.test/shared";

struct DuplicatePackageProgram {
    packages: Vec<Arc<RuntimeExecutionPackage>>,
    task_routes: HashMap<String, skiff_runtime_linked_program::ExecutableAddr>,
    link_overlay: LinkOverlay,
    types: RuntimeTypeContext,
}

impl DuplicatePackageProgram {
    fn new() -> Self {
        Self {
            packages: vec![
                crate::test_support::runtime_execution_package_fixture(
                    PACKAGE_ID,
                    0,
                    Vec::new(),
                    Default::default(),
                ),
                crate::test_support::runtime_execution_package_fixture(
                    PACKAGE_ID,
                    1,
                    Vec::new(),
                    Default::default(),
                ),
            ],
            task_routes: HashMap::new(),
            link_overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn projection(&self) -> EvalProgramProjection<'_> {
        EvalProgramProjection::new(
            "skiff.test/service",
            &[],
            &self.packages,
            &self.task_routes,
            &self.link_overlay,
            &self.types,
        )
    }
}

#[test]
fn duplicate_package_id_different_build_allows_plain_data_hook_construction() {
    let program = DuplicatePackageProgram::new();

    EvalRecoverableBehaviorHooks::new(program.projection(), "artifact:test", "build:test")
        .expect("plain-data hook construction must not eagerly validate package owner lookup");
}
