use std::{future::Future, sync::Arc};

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

    fn wait_for_wake(&mut self) -> impl Future<Output = RequestResult<Self::Wake>> + Send + '_;
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

    fn wait_for_wake(&mut self) -> impl Future<Output = RequestResult<Self::Wake>> + Send + '_ {
        async move { request_runner::BytecodeRequestExecution::wait_pending_wake(self).await }
    }
}

pub(super) struct DrivenBytecodeRequest {
    pub(super) result: RequestResult<BoundaryResponse>,
    pub(super) execution: Option<BytecodeRequestExecution>,
}

pub(super) async fn drive_bytecode_request(
    input: BytecodeRequestExecutionInput,
    response_events: Arc<dyn ResponseEventSink>,
) -> DrivenBytecodeRequest {
    let mut execution = match request_runner::start_runtime_bytecode_request(input, response_events)
    {
        Ok(execution) => execution,
        Err(error) => {
            return DrivenBytecodeRequest {
                result: Err(error),
                execution: None,
            }
        }
    };
    let result = drive_bytecode_request_with(&mut execution).await;
    DrivenBytecodeRequest {
        result,
        execution: Some(execution),
    }
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
                let wake = if let Some(wake) = execution.take_pending_wake() {
                    wake
                } else {
                    execution.wait_for_wake().await?
                };
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
    use std::{collections::VecDeque, sync::Mutex};

    use skiff_runtime_request::{ResponseEnd, ResponseEvent};

    use super::*;

    struct FakeExecution {
        wakes: Arc<Mutex<VecDeque<usize>>>,
        resumed: Arc<Mutex<Vec<usize>>>,
    }

    impl ResumableBytecodeRequest for FakeExecution {
        type Wake = usize;

        fn run(&mut self) -> RequestResult<BytecodeRequestRunOutcome> {
            if !self.resumed.lock().unwrap().is_empty() {
                return Err(RequestError::Decode(
                    "fake execution must be resumed after parking".to_string(),
                ));
            }
            Ok(BytecodeRequestRunOutcome::Parked)
        }

        fn take_pending_wake(&mut self) -> Option<Self::Wake> {
            self.wakes.lock().unwrap().pop_front()
        }

        fn resume(&mut self, wake: Self::Wake) -> RequestResult<BytecodeRequestRunOutcome> {
            let mut resumed = self.resumed.lock().unwrap();
            resumed.push(wake);
            if resumed.len() < 2 {
                Ok(BytecodeRequestRunOutcome::Parked)
            } else {
                Ok(BytecodeRequestRunOutcome::Complete(
                    BoundaryResponse::payload(b"done".to_vec()),
                ))
            }
        }

        fn wait_for_wake(&mut self) -> impl Future<Output = RequestResult<Self::Wake>> + Send + '_ {
            async move {
                loop {
                    if let Some(wake) = self.take_pending_wake() {
                        return Ok(wake);
                    }
                    tokio::task::yield_now().await;
                }
            }
        }
    }

    #[tokio::test]
    async fn host_driver_consumes_pending_wakes_until_terminal() {
        let wakes = Arc::new(Mutex::new(VecDeque::from([1, 2])));
        let resumed = Arc::new(Mutex::new(Vec::new()));
        let mut execution = FakeExecution {
            wakes: Arc::clone(&wakes),
            resumed: Arc::clone(&resumed),
        };

        let response = drive_bytecode_request_with(&mut execution).await.unwrap();

        assert_eq!(*resumed.lock().unwrap(), [1, 2]);
        assert!(matches!(
            response,
            BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))
                if payload == b"done"
        ));
    }

    #[tokio::test]
    async fn host_driver_waits_for_late_pending_wake() {
        let wakes = Arc::new(Mutex::new(VecDeque::new()));
        let resumed = Arc::new(Mutex::new(Vec::new()));
        let mut execution = FakeExecution {
            wakes: Arc::clone(&wakes),
            resumed: Arc::clone(&resumed),
        };

        let request = tokio::spawn(async move {
            let response = drive_bytecode_request_with(&mut execution).await.unwrap();
            (response, execution)
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        wakes.lock().unwrap().extend([1, 2]);

        let (response, execution) = request.await.unwrap();
        assert_eq!(*execution.resumed.lock().unwrap(), [1, 2]);
        assert!(matches!(
            response,
            BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))
                if payload == b"done"
        ));
    }
}
