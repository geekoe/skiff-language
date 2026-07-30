use std::sync::Arc;

use skiff_artifact_model::PublicationResourceRef;
use skiff_runtime_model::addr::UnitAddr;

use super::*;

struct ExactResourceCapability {
    expected_owner: UnitAddr,
    expected_path: &'static str,
    resource: LoadedPublicationResource,
}

impl NativeResourceCapability for ExactResourceCapability {
    fn lookup_resource<'a>(
        &'a self,
        owner: &UnitAddr,
        path: &str,
    ) -> std::result::Result<Option<&'a LoadedPublicationResource>, RuntimeProgramResourceLookupError>
    {
        assert_eq!(owner, &self.expected_owner);
        assert_eq!(path, self.expected_path);
        Ok(Some(&self.resource))
    }
}

struct OutOfBoundsResourceCapability;

impl NativeResourceCapability for OutOfBoundsResourceCapability {
    fn lookup_resource<'a>(
        &'a self,
        owner: &UnitAddr,
        path: &str,
    ) -> std::result::Result<Option<&'a LoadedPublicationResource>, RuntimeProgramResourceLookupError>
    {
        assert_eq!(owner, &UnitAddr::Package(7));
        assert_eq!(path, "prompts/system.md");
        Err(RuntimeProgramResourceLookupError::PackageSlotOutOfBounds {
            slot: 7,
            package_count: 1,
        })
    }
}

#[test]
fn resource_lookup_forwards_exact_owner_and_path() {
    let resource = loaded_resource("prompts/system.md", b"system");
    let context = ExactResourceCapability {
        expected_owner: UnitAddr::Package(3),
        expected_path: "prompts/system.md",
        resource,
    };
    let invocation = resource_invocation(UnitAddr::Package(3));
    let path = PublicationResourcePath::parse("prompts/system.md").unwrap();

    let actual = lookup_resource(&context, &invocation, &path)
        .expect("exact package resource lookup should succeed")
        .expect("fixture resource should exist");

    assert!(std::ptr::eq(actual, &context.resource));
}

#[test]
fn resource_lookup_maps_package_slot_failure_to_invalid_artifact() {
    let invocation = resource_invocation(UnitAddr::Package(7));
    let path = PublicationResourcePath::parse("prompts/system.md").unwrap();

    let error = lookup_resource(&OutOfBoundsResourceCapability, &invocation, &path)
        .expect_err("out-of-bounds package owner must fail closed");

    assert!(matches!(
        error,
        RuntimeError::InvalidArtifact(ref message)
            if message == "resource owner package slot 7 is out of bounds for 1 packages"
    ));
}

#[test]
fn empty_resource_capability_preserves_service_and_package_semantics() {
    assert!(
        ().lookup_resource(&UnitAddr::Service, "missing")
            .unwrap()
            .is_none()
    );

    assert!(matches!(
        ().lookup_resource(&UnitAddr::Package(2), "missing"),
        Err(RuntimeProgramResourceLookupError::PackageSlotOutOfBounds {
            slot: 2,
            package_count: 0,
        })
    ));
}

fn resource_invocation(owner: UnitAddr) -> RuntimeNativeInvocation {
    RuntimeNativeInvocation::new(
        "std.resource.text".to_string(),
        "std.resource.text",
        None,
        None,
        Some(owner),
    )
}

fn loaded_resource(path: &str, bytes: &[u8]) -> LoadedPublicationResource {
    LoadedPublicationResource {
        meta: PublicationResourceRef {
            path: path.to_string(),
            sha256: format!("test-sha256:{}", bytes.len()),
            byte_len: bytes.len() as u64,
            content_type: Some("text/plain".to_string()),
            artifact_path: Some(format!("resources/{path}")),
        },
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
    }
}
