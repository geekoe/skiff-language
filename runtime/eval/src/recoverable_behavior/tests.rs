use std::{collections::HashMap, sync::Arc};

use skiff_runtime_linked_program::{
    LinkOverlay, RuntimeExecutionPackage, RuntimeTypeContext, UnitAddr,
};

use super::{local_concrete_owner, EvalRecoverableBehaviorHooks};
use crate::{error::RuntimeError, invocation::EvalProgramProjection};

const PACKAGE_ID: &str = "skiff.test/shared";

struct DuplicatePackageProgram {
    packages: Vec<Arc<RuntimeExecutionPackage>>,
    spawn_routes: HashMap<String, skiff_runtime_linked_program::ExecutableAddr>,
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
            spawn_routes: HashMap::new(),
            link_overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn projection(&self) -> EvalProgramProjection<'_> {
        EvalProgramProjection::new(
            "skiff.test/service",
            &[],
            &self.packages,
            &self.spawn_routes,
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

#[test]
fn duplicate_package_id_fails_closed_when_package_local_concrete_owner_is_needed() {
    let program = DuplicatePackageProgram::new();
    let projection = super::RuntimeExecutionProjection::from(program.projection());

    let result = local_concrete_owner(&projection, &UnitAddr::Package(0));

    match result {
        Err(RuntimeError::InvalidArtifact(message)) => assert!(
            message.contains("package id skiff.test/shared is ambiguous"),
            "unexpected invalid artifact message: {message}"
        ),
        Err(error) => panic!("expected invalid artifact error, got {error}"),
        Ok(owner) => panic!("ambiguous package owner must fail closed, got {owner:?}"),
    }
}
