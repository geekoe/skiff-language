use std::future::Future;

use skiff_runtime_boundary::{
    binary::decode_payload_plan,
    payload::{PayloadBoundary, PayloadBoundaryKind},
};
use skiff_runtime_capability_context::{OutboundRequestLease, OutboundResponseReceiver};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::{
    outbound_router_response_into_result, stream_sink_is_cancelled, Env, OutboundResponse,
    OutboundServiceContext, OutboundServiceDispatch, OutboundServiceResponse, Result, RuntimeError,
};
use crate::runtime_ops::runtime_coerce_required_plan;

pub(crate) enum PreparedOutboundServiceCall {
    Ready(RuntimeValue),
    ExternalWait(PreparedOutboundUnaryOperation),
}

pub(crate) struct PreparedOutboundUnaryOperation {
    context: OutboundServiceContext,
    dispatch: OutboundServiceDispatch,
    lease: OutboundRequestLease,
    receiver: OutboundResponseReceiver,
}

pub(crate) struct OutboundServiceUnaryCompletion {
    dispatch: OutboundServiceDispatch,
    outcome: Result<OutboundServiceResponse>,
}

impl PreparedOutboundUnaryOperation {
    pub(super) fn new(
        context: OutboundServiceContext,
        dispatch: OutboundServiceDispatch,
        lease: OutboundRequestLease,
        receiver: OutboundResponseReceiver,
    ) -> Self {
        Self {
            context,
            dispatch,
            lease,
            receiver,
        }
    }

    pub(crate) fn into_wait(
        self,
    ) -> impl Future<Output = OutboundServiceUnaryCompletion> + Send + 'static {
        async move {
            let Self {
                context,
                dispatch,
                lease,
                receiver,
            } = self;
            let outcome = await_outbound_response(context, &dispatch, lease, receiver).await;
            OutboundServiceUnaryCompletion { dispatch, outcome }
        }
    }
}

impl OutboundServiceUnaryCompletion {
    pub(crate) fn finalize(self, heap: &mut RequestHeap, env: &Env) -> Result<RuntimeValue> {
        let checkpoint = heap.checkpoint();
        let result = self.finalize_inner(heap, env);
        if result.is_err() {
            heap.rollback_to_checkpoint(checkpoint);
        }
        result
    }

    fn finalize_inner(self, heap: &mut RequestHeap, env: &Env) -> Result<RuntimeValue> {
        let response = self.outcome?;
        let boundary = PayloadBoundary::cross_service(
            PayloadBoundaryKind::InboundServiceCall,
            self.dispatch.service_ref(),
        );
        let value = decode_payload_plan(
            &response.payload,
            &self.dispatch.response_plan,
            &boundary,
            heap,
        )?;
        let value = runtime_coerce_required_plan(
            &value,
            &self.dispatch.response_plan,
            &format!("{} response", self.dispatch.target),
            heap,
        )?;
        if stream_sink_is_cancelled(env) {
            return Err(RuntimeError::Cancelled);
        }
        Ok(value)
    }
}

async fn await_outbound_response(
    context: OutboundServiceContext,
    dispatch: &OutboundServiceDispatch,
    lease: OutboundRequestLease,
    mut receiver: OutboundResponseReceiver,
) -> Result<OutboundServiceResponse> {
    let timeout = context.effective_timeout_ms(dispatch.timeout_ms);
    let response = match context
        .receive_response(&lease, &dispatch.target, &mut receiver, timeout)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            lease.cancel("response_channel_closed");
            return Err(error);
        }
    };
    match response {
        response @ (OutboundResponse::End { .. }
        | OutboundResponse::FixedServiceFailure(_)
        | OutboundResponse::Error(_)) => {
            lease.complete();
            outbound_router_response_into_result(response, &dispatch.target)
        }
        other => {
            lease.cancel("unexpected_stream_response");
            Err(RuntimeError::ProviderUnavailable {
                target: dispatch.target.clone(),
                reason: format!("unary outbound service call received {}", other.kind()),
            })
        }
    }
}
