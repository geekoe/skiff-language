use super::{
    fixture::Capability,
    host_harness::{HostResponse, RuntimeHostHarness},
    stages::published_positive,
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

async fn drive(capability: Capability, prefix: &str) -> HostResponse {
    let fixture = published_positive(capability, prefix);
    let path = capability_path(capability);
    let mut host = RuntimeHostHarness::start(prefix, fixture).await;
    let request_id = format!("phase-6-{prefix}-{capability:?}");
    let mode = if capability == Capability::Service {
        "unary"
    } else {
        "serverStream"
    };
    let body = if capability == Capability::Service {
        b"7".as_slice()
    } else {
        b"phase6".as_slice()
    };
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
