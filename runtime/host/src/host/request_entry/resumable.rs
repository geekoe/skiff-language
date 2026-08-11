use std::sync::Arc;

use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, BytecodeRequestExecution,
    BytecodeRequestExecutionInput, BytecodeRequestPendingWake, BytecodeRequestRunOutcome,
    RequestError, RequestResult, ResponseEventSink, ResponseStreamEvent,
};

#[allow(clippy::result_large_err)]
trait ResumableBytecodeRequest {
    type Wake;

    fn run(&mut self) -> RequestResult<BytecodeRequestRunOutcome>;

    fn take_pending_wake(&mut self) -> Option<Self::Wake>;

    fn resume(&mut self, wake: Self::Wake) -> RequestResult<BytecodeRequestRunOutcome>;
}

impl ResumableBytecodeRequest for BytecodeRequestExecution {
    type Wake = BytecodeRequestPendingWake;

    fn run(&mut self) -> RequestResult<BytecodeRequestRunOutcome> {
        self.run()
    }

    fn take_pending_wake(&mut self) -> Option<Self::Wake> {
        self.take_pending_wake()
    }

    fn resume(&mut self, wake: Self::Wake) -> RequestResult<BytecodeRequestRunOutcome> {
        self.resume(wake)
    }
}

pub(super) async fn drive_bytecode_request(
    input: BytecodeRequestExecutionInput,
    response_events: Arc<dyn ResponseEventSink>,
) -> RequestResult<BoundaryResponse> {
    let mut execution = request_runner::start_runtime_bytecode_request(input, response_events)?;
    drive_bytecode_request_with(&mut execution).await
}

async fn drive_bytecode_request_with<R>(execution: &mut R) -> RequestResult<BoundaryResponse>
where
    R: ResumableBytecodeRequest + ?Sized,
{
    let mut outcome = execution.run()?;
    loop {
        match outcome {
            BytecodeRequestRunOutcome::Complete(response) => return Ok(response),
            BytecodeRequestRunOutcome::Parked => {
                tokio::task::yield_now().await;
                let wake = execution.take_pending_wake().ok_or_else(|| {
                    RequestError::Decode(
                        "bytecode request parked without a ready pending wake".to_string(),
                    )
                })?;
                outcome = execution.resume(wake)?;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RejectingResponseEventSink;

impl ResponseEventSink for RejectingResponseEventSink {
    fn send_stream_event(
        &self,
        _request_id: &str,
        _event: ResponseStreamEvent,
    ) -> RequestResult<()> {
        Err(RequestError::Unsupported(
            "bytecode response stream is not configured for this host ingress".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use skiff_runtime_request::{ResponseEnd, ResponseEvent};

    use super::*;

    struct FakeExecution {
        wakes: VecDeque<usize>,
        resumed: Vec<usize>,
    }

    impl ResumableBytecodeRequest for FakeExecution {
        type Wake = usize;

        fn run(&mut self) -> RequestResult<BytecodeRequestRunOutcome> {
            if !self.resumed.is_empty() {
                return Err(RequestError::Decode(
                    "fake execution must be resumed after parking".to_string(),
                ));
            }
            Ok(BytecodeRequestRunOutcome::Parked)
        }

        fn take_pending_wake(&mut self) -> Option<Self::Wake> {
            self.wakes.pop_front()
        }

        fn resume(&mut self, wake: Self::Wake) -> RequestResult<BytecodeRequestRunOutcome> {
            self.resumed.push(wake);
            if self.resumed.len() < 2 {
                Ok(BytecodeRequestRunOutcome::Parked)
            } else {
                Ok(BytecodeRequestRunOutcome::Complete(
                    BoundaryResponse::payload(b"done".to_vec()),
                ))
            }
        }
    }

    #[tokio::test]
    async fn host_driver_consumes_pending_wakes_until_terminal() {
        let mut execution = FakeExecution {
            wakes: VecDeque::from([1, 2]),
            resumed: Vec::new(),
        };

        let response = drive_bytecode_request_with(&mut execution).await.unwrap();

        assert_eq!(execution.resumed, [1, 2]);
        assert!(matches!(
            response,
            BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))
                if payload == b"done"
        ));
    }
}
