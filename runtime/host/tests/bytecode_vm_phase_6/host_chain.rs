use super::{
    fixture::Capability,
    host_harness::{HostError, HostResponse, RuntimeHostHarness},
    stages::{published_interface_local_named, published_positive},
};

pub async fn scheduler_to_request(capability: Capability, prefix: &str) {
    let response = drive(capability, prefix).await;
    assert_eq!(
        response.status, 200,
        "Phase 6 capability did not produce its success terminal"
    );
}

pub async fn request_to_terminal(capability: Capability, prefix: &str) {
    let response = drive(capability, prefix).await;
    assert!(
        response.status == 200 || response.status == 207,
        "Phase 6 capability terminal status was {response_status}",
        response_status = response.status
    );
    assert!(
        !response.chunks.is_empty(),
        "Phase 6 capability terminal must expose observable body output"
    );
}

pub async fn interface_local_named_to_terminal(
    directory: &str,
    package_id: &str,
    prefix: &str,
) -> HostResponse {
    let fixture = published_interface_local_named(directory, package_id, prefix);
    drive_fixture(
        fixture,
        named_interface_path(directory),
        "unary",
        b"7",
        prefix,
    )
    .await
}

pub async fn interface_local_named_throw_terminal(
    directory: &str,
    package_id: &str,
    prefix: &str,
) -> HostError {
    let fixture = published_interface_local_named(directory, package_id, prefix);
    let mut host = RuntimeHostHarness::start(prefix, fixture).await;
    let request_id = format!("phase-6-{prefix}");
    host.send_http_request(&request_id, named_interface_path(directory), "unary", b"7")
        .await;
    let error = host.error(&request_id).await;
    host.close().await;
    error
}

async fn drive(capability: Capability, prefix: &str) -> HostResponse {
    let fixture = published_positive(capability, prefix);
    let path = capability_path(capability);
    let unary_json = matches!(
        capability,
        Capability::Service
            | Capability::InterfaceLocal
            | Capability::Recoverable
            | Capability::Db
            | Capability::Task
            | Capability::Actor
    );
    let mode = if unary_json { "unary" } else { "serverStream" };
    let body = if unary_json {
        b"7".as_slice()
    } else {
        b"phase6".as_slice()
    };
    drive_fixture(fixture, path, mode, body, prefix).await
}

async fn drive_fixture(
    fixture: super::fixture::PublishedFixture,
    path: &str,
    mode: &str,
    body: &[u8],
    prefix: &str,
) -> HostResponse {
    let mut host = RuntimeHostHarness::start(prefix, fixture).await;
    let request_id = format!("phase-6-{prefix}");
    host.send_http_request(&request_id, path, mode, body).await;
    let response = host.response(&request_id).await;
    host.close().await;
    response
}

fn capability_path(capability: Capability) -> &'static str {
    match capability {
        Capability::Service => "/phase-6/service",
        Capability::InterfaceLocal | Capability::InterfaceRemote => "/phase-6/interface",
        Capability::Callback => "/phase-6/callback",
        Capability::Recoverable => "/phase-6/recoverable",
        Capability::Db => "/phase-6/db",
        Capability::Task => "/phase-6/task",
        Capability::Actor => "/phase-6/actor",
        Capability::Containment => "/phase-6/containment",
    }
}

fn named_interface_path(directory: &str) -> &'static str {
    match directory {
        "interface-local-success" => "/phase-6/interface-local",
        "interface-local-throw" => "/phase-6/interface-local-throw",
        "interface-local-pending" => "/phase-6/interface-local-pending",
        "interface-local-bad-slot" => "/phase-6/interface-local-bad-slot",
        "interface-local-bad-carrier" => "/phase-6/interface-local-bad-carrier",
        "interface-local-bad-signature" => "/phase-6/interface-local-bad-signature",
        other => panic!("unknown Phase 6 interface-local fixture directory {other}"),
    }
}
