use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryFeatureUnavailableReason, BoundaryOperationContract,
    BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef,
};
use skiff_runtime_model::runtime_value::RuntimeValue;

use super::{
    artifacts::{ProjectedFixture, TypedExecutionContract},
    runtime::TypedExecutionRuntime,
    scenario::TypedExecutionFixture,
};

#[tokio::test]
async fn typed_execution_async_stream_cancel_reaches_owned_provider_future_full_chain() {
    let fixture = TypedExecutionFixture::admit_contract(TypedExecutionContract::new(
        async_unary_contract(),
        BTreeMap::new(),
    ))
    .await;
    let provider = fixture.resolve_provider();
    let receiver_id = fixture
        .eval_target
        .activation_context()
        .activation_id()
        .clone();
    assert_ne!(provider.provider_activation().activation_id(), &receiver_id);

    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let generation = context
        .runtime_assembly_target()
        .unwrap()
        .request_activation()
        .generation();
    let mut heap = context.request_heap();

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("fixture provider has no body, but the owned provider future must be reached");

    assert!(
        error.to_string().contains("provide missing block entry"),
        "service call stopped before the exact provider executable: {error}"
    );
    assert_eq!(
        fixture.eval_target.activation_context().activation_id(),
        &receiver_id,
        "caller must remain in the receiver activation after provider completion"
    );
    assert_eq!(
        provider.provider_request().generation(),
        generation,
        "provider future must retain the explicit request generation"
    );
}

#[tokio::test]
async fn typed_execution_async_stream_cancel_spawns_server_stream_from_admitted_target() {
    let baseline = crate::capability_context::stream_runtime_streams_active();
    {
        let fixture = TypedExecutionFixture::admit_contract(TypedExecutionContract::new(
            server_stream_contract(),
            BTreeMap::new(),
        ))
        .await;
        let runtime = TypedExecutionRuntime::new(
            &fixture
                .eval_target
                .activation_context()
                .identity()
                .deployment
                .service_id,
        );
        let interpreter = runtime.interpreter();
        let context = runtime.context(&interpreter, &fixture.eval_target);
        let mut heap = context.request_heap();

        let result = interpreter
            .execute_runtime_assembly_addr(
                context,
                &mut heap,
                &fixture.consumer_executable_addr(0),
                Vec::new(),
            )
            .await
            .expect("server stream should reach the async lane from the admitted call target");
        assert_eq!(result, RuntimeValue::Null);
    }

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while crate::capability_context::stream_runtime_streams_active() != baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the full-chain runtime must close its stream registry and lifetime lease");
}

#[test]
fn typed_execution_async_stream_cancel_rejects_unsupported_descriptor_before_provider() {
    let mut contract = async_unary_contract();
    contract.cancellation = BoundaryCancellationContract::Unsupported {
        reason: BoundaryFeatureUnavailableReason::UnknownSemantics,
    };
    let rejected = std::panic::catch_unwind(|| {
        ProjectedFixture::new(TypedExecutionContract::new(contract, BTreeMap::new()))
    });
    assert!(
        rejected.is_err(),
        "unsupported cancellation descriptor must fail during typed projection"
    );
}

fn async_unary_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: detached_plan(BoundaryValueLifetime::Call),
        },
        errors: BoundaryErrorContract::None,
        stream: BoundaryStreamContract::Unary,
        cancellation: BoundaryCancellationContract::Cooperative,
        callbacks: BoundaryCallbackContract::None,
        may_suspend: true,
        effect_guarantee: detached_effect_guarantee(),
    }
}

fn server_stream_contract() -> BoundaryOperationContract {
    let mut contract = async_unary_contract();
    contract.stream = BoundaryStreamContract::ServerStream {
        item_type: ContractTypeRef::builtin("bool"),
        item_value_plan: detached_plan(BoundaryValueLifetime::Stream),
    };
    contract
}

fn detached_plan(lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime,
    }
}

fn detached_effect_guarantee() -> BoundaryEffectGuarantee {
    BoundaryEffectGuarantee {
        detached_parameters: true,
        detached_return: true,
        detached_error: true,
        no_caller_reachable_mutation: true,
        no_caller_value_escape: true,
        no_same_heap_identity: true,
    }
}
