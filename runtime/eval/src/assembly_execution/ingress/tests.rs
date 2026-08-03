use skiff_runtime_capability_context::RequestPayloadContext;

use super::*;
use crate::assembly_execution::ordinary::tests::{
    service_error_consumer::{ConsumerTopology, ProviderFailureKind, ServiceErrorConsumerFixture},
    test_runtime,
};

#[tokio::test]
async fn ingress_hands_fixed_failure_up_without_importing_an_external_caller() {
    for kind in [
        ProviderFailureKind::PublicRecord,
        ProviderFailureKind::Private,
    ] {
        let fixture = ServiceErrorConsumerFixture::new(kind, ConsumerTopology::OneHop, true, false);
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let eval_target = fixture.terminal_eval_target();
        let target = fixture.ingress_target(&eval_target);
        let context = fixture.execution_context(&interpreter, eval_target);
        let mut access = HeapAccess::private(RequestHeap::default());
        let request = RequestPayloadContext::new("fixture-ingress", &[], None);

        let error = dispatch_ingress_via_in_process_boundary(
            &interpreter,
            context,
            &mut access,
            target,
            &request,
        )
        .await
        .expect_err("provider failure must remain a fixed ingress carrier");
        let RuntimeError::FixedServiceFailure(error) = error else {
            panic!("ingress must not manufacture an external caller UserException");
        };
        assert!(
            access.heap_mut().is_empty(),
            "ingress must not allocate a caller-local exception or retain the provider heap"
        );
        let bytes = String::from_utf8_lossy(error.encoded_bytes());
        for private in [
            "source:provider-private-path",
            "throwPrivate",
            "PrivateFault",
            "provider-private-secret",
        ] {
            assert!(
                !bytes.contains(private),
                "fixed ingress bytes leaked provider-local fact {private}"
            );
        }
    }
}
