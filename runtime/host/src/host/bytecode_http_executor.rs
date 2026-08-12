use std::sync::Arc;

use serde_json::Value;
use skiff_runtime_boundary::value::bytes_payload;
use skiff_runtime_capability_context::{CancellationSignals, CancellationToken};
use skiff_runtime_request::{
    BytecodeHttpExecutor, BytecodeHttpStream, BytecodeHttpStreamEvent, HttpNameValue,
};

use crate::{
    capability_context::{HttpClientCapabilityContext, HttpRuntimeOptions},
    host::http_runtime::{
        open_body_stream_with_cancellation_and_options, request_with_cancellation_and_options,
    },
};

pub(crate) struct RuntimeBytecodeHttpExecutor {
    context: HttpClientCapabilityContext,
    options: HttpRuntimeOptions,
}

impl RuntimeBytecodeHttpExecutor {
    pub(crate) fn new(context: HttpClientCapabilityContext, options: HttpRuntimeOptions) -> Self {
        Self { context, options }
    }

    fn stream_with_test_effects(&self, input: Value) -> Result<BytecodeHttpStream, String> {
        let context = self.context.clone();
        let (head_tx, head_rx) =
            std::sync::mpsc::channel::<Result<(u16, Vec<HttpNameValue>), String>>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<BytecodeHttpStreamEvent>();
        let stream_cancellation = CancellationToken::new();
        let thread_stream_cancellation = stream_cancellation.clone();

        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = head_tx.send(Err(error.to_string()));
                    return;
                }
            };
            let result: Result<(), ()> = runtime.block_on(async move {
                let handle = match context
                    .dispatch_http_stream(
                        &input,
                        Some(&skiff_runtime_model::type_plan::builtins::leaf_bytes_plan()),
                    )
                    .await
                {
                    Ok(handle) => handle,
                    Err(error) => {
                        let _ = head_tx.send(Err(error.to_string()));
                        return Ok(());
                    }
                };
                let status = handle
                    .get("status")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| "test-effect HTTP stream is missing status".to_string());
                let headers = handle
                    .get("headers")
                    .cloned()
                    .ok_or_else(|| "test-effect HTTP stream is missing headers".to_string())
                    .and_then(|value| {
                        serde_json::from_value::<Vec<HttpNameValue>>(value)
                            .map_err(|error| error.to_string())
                    });
                let body = handle
                    .get("body")
                    .cloned()
                    .ok_or_else(|| "test-effect HTTP stream is missing body".to_string());
                let (status, headers, body) = match (status, headers, body) {
                    (Ok(status), Ok(headers), Ok(body)) => (status as u16, headers, body),
                    (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                        let _ = head_tx.send(Err(error));
                        return Ok(());
                    }
                };
                if head_tx.send(Ok((status, headers))).is_err() {
                    return Ok(());
                }
                let stream_runtime = context.stream_runtime().clone();
                loop {
                    let cancellation =
                        CancellationSignals::from_tokens([thread_stream_cancellation.clone()]);
                    match stream_runtime
                        .next_with_cancellation(&body, &[], &cancellation)
                        .await
                    {
                        Ok(skiff_runtime_capability_context::StreamPoll::Item(item)) => {
                            let Some(bytes) = bytes_payload(&item) else {
                                let _ = event_tx.send(BytecodeHttpStreamEvent::Error(
                                    "test-effect HTTP stream chunk is missing bytes payload"
                                        .to_string(),
                                ));
                                break;
                            };
                            if event_tx
                                .send(BytecodeHttpStreamEvent::Chunk(bytes))
                                .is_err()
                            {
                                thread_stream_cancellation.cancel();
                                break;
                            }
                        }
                        Ok(skiff_runtime_capability_context::StreamPoll::InternalItem(_)) => {
                            let _ = event_tx.send(BytecodeHttpStreamEvent::Error(
                                "test-effect HTTP stream returned an internal item".to_string(),
                            ));
                            break;
                        }
                        Ok(skiff_runtime_capability_context::StreamPoll::End) => {
                            let _ = event_tx.send(BytecodeHttpStreamEvent::End);
                            break;
                        }
                        Err(error) => {
                            let _ =
                                event_tx.send(BytecodeHttpStreamEvent::Error(error.to_string()));
                            break;
                        }
                    }
                }
                Ok(())
            });
            let _ = result;
        });

        let (status, headers) = head_rx
            .recv()
            .map_err(|_| "test-effect HTTP stream head worker closed".to_string())??;
        Ok(BytecodeHttpStream {
            status,
            headers,
            events: event_rx,
            cancel: Box::new(move || {
                stream_cancellation.cancel();
            }),
        })
    }
}

impl BytecodeHttpExecutor for RuntimeBytecodeHttpExecutor {
    fn request(
        &self,
        input: Value,
        use_test_effects: bool,
        allow_unsafe_targets: bool,
    ) -> Result<Value, String> {
        let context = self.context.clone();
        let mut options = self.options.clone();
        if allow_unsafe_targets {
            options = options.with_allow_unsafe_targets(true);
        }
        let deadline_ms = self.context.http().deadline_ms();
        let response_max_bytes = self.context.http().response_max_bytes();
        let cancellation = self.context.http().cancellation_token();

        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(async move {
                if use_test_effects {
                    context
                        .dispatch_http_request(&input)
                        .await
                        .map_err(|error| error.to_string())
                } else {
                    request_with_cancellation_and_options(
                        &input,
                        deadline_ms,
                        response_max_bytes,
                        CancellationSignals::from_tokens([cancellation]),
                        options,
                    )
                    .await
                    .map_err(|error| error.to_string())
                }
            })
        });
        worker
            .join()
            .map_err(|_| "bytecode HTTP worker panicked".to_string())?
    }

    fn stream(
        &self,
        input: Value,
        use_test_effects: bool,
        allow_unsafe_targets: bool,
    ) -> Result<BytecodeHttpStream, String> {
        if use_test_effects {
            return self.stream_with_test_effects(input);
        }

        let mut options = self.options.clone();
        if allow_unsafe_targets {
            options = options.with_allow_unsafe_targets(true);
        }
        let deadline_ms = self.context.http().deadline_ms();
        let response_max_bytes = self.context.http().response_max_bytes();
        let cancellation = self.context.http().cancellation_token();
        let stream_cancellation = CancellationToken::new();
        let thread_stream_cancellation = stream_cancellation.clone();

        let (head_tx, head_rx) =
            std::sync::mpsc::channel::<Result<(u16, Vec<HttpNameValue>), String>>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<BytecodeHttpStreamEvent>();

        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = head_tx.send(Err(error.to_string()));
                    return;
                }
            };
            let result: Result<(), ()> = runtime.block_on(async move {
                let mut body = match open_body_stream_with_cancellation_and_options(
                    &input,
                    deadline_ms,
                    CancellationSignals::from_tokens([
                        cancellation,
                        thread_stream_cancellation.clone(),
                    ]),
                    response_max_bytes,
                    options,
                )
                .await
                {
                    Ok(body) => body,
                    Err(error) => {
                        let _ = head_tx.send(Err(error.to_string()));
                        return Ok(());
                    }
                };
                let (status, headers_value) = body.handle_metadata();
                let headers = match serde_json::from_value::<Vec<HttpNameValue>>(headers_value) {
                    Ok(headers) => headers,
                    Err(error) => {
                        let _ = head_tx.send(Err(error.to_string()));
                        return Ok(());
                    }
                };
                if head_tx.send(Ok((status, headers))).is_err() {
                    return Ok(());
                }
                loop {
                    match body.next_body_chunk().await {
                        Ok(Some(chunk)) => {
                            let Some(bytes) = bytes_payload(&chunk) else {
                                let _ = event_tx.send(BytecodeHttpStreamEvent::Error(
                                    "HTTP stream chunk is missing bytes payload".to_string(),
                                ));
                                break;
                            };
                            if event_tx
                                .send(BytecodeHttpStreamEvent::Chunk(bytes))
                                .is_err()
                            {
                                thread_stream_cancellation.cancel();
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = event_tx.send(BytecodeHttpStreamEvent::End);
                            break;
                        }
                        Err(error) => {
                            let _ =
                                event_tx.send(BytecodeHttpStreamEvent::Error(error.to_string()));
                            break;
                        }
                    }
                }
                Ok(())
            });
            let _ = result;
        });

        let (status, headers) = head_rx
            .recv()
            .map_err(|_| "bytecode HTTP stream head worker closed".to_string())??;
        Ok(BytecodeHttpStream {
            status,
            headers,
            events: event_rx,
            cancel: Box::new(move || {
                stream_cancellation.cancel();
            }),
        })
    }
}

pub(crate) fn build_bytecode_http_executor(
    context: HttpClientCapabilityContext,
    options: HttpRuntimeOptions,
) -> Arc<dyn BytecodeHttpExecutor> {
    Arc::new(RuntimeBytecodeHttpExecutor::new(context, options))
}
