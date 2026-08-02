//! W-http real-socket probes: server-stream mapping, sequencing, ceiling and
//! backpressure over the fake dispatcher.

mod http_common;

#[cfg(test)]
mod tests {

    use std::sync::Arc;
    use std::time::Duration;

    use bytes::Bytes;
    use skiff_router::http::fake::{FakeDispatchPlan, FakeHttpDispatcher, FakeStreamEvent};
    use skiff_router::http::{start_http_gateway, HttpGatewayServer, HttpGatewayServerOptions};
    use skiff_runtime_transport::cancel_reason::RequestCancelReason;

    use crate::http_common::{
        fixture_epoch, fixture_resolver, send_request, service_headers, StreamClient,
    };

    async fn start_stream_server(
        dispatcher: FakeHttpDispatcher,
        max_response_bytes: usize,
        backpressure_drain_timeout: Duration,
        channel_capacity: usize,
    ) -> (HttpGatewayServer, FakeHttpDispatcher) {
        let dispatcher = Arc::new(dispatcher);
        let options = HttpGatewayServerOptions::new(
            "127.0.0.1:0".parse().expect("bind"),
            1024 * 1024,
            max_response_bytes,
        );
        let options = HttpGatewayServerOptions {
            backpressure_drain_timeout,
            stream_channel_capacity: channel_capacity,
            ..options
        };
        let server = start_http_gateway(
            options,
            fixture_epoch(),
            fixture_resolver(),
            dispatcher.clone(),
        )
        .await
        .expect("start http gateway");
        (server, (*dispatcher).clone())
    }

    fn stream_chunks(count: usize, payload: &'static [u8]) -> Vec<FakeStreamEvent> {
        let mut events = Vec::new();
        for index in 0..count {
            events.push(FakeStreamEvent::Chunk {
                seq: index as u64,
                payload: Bytes::from_static(payload),
            });
        }
        events
    }

    fn stream_chunks_size(count: usize, size: usize) -> Vec<FakeStreamEvent> {
        let payload = Bytes::from(vec![0x61u8; size]);
        (0..count)
            .map(|index| FakeStreamEvent::Chunk {
                seq: index as u64,
                payload: payload.clone(),
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_roundtrip_preserves_chunk_order() {
        let mut events = vec![FakeStreamEvent::Start {
            status: 200,
            headers: vec![("content-type".to_string(), "text/plain".to_string())],
        }];
        events.extend(stream_chunks(3, b"x"));
        events.push(FakeStreamEvent::End);
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream { events }]);
        let (server, mut fake) = start_stream_server(fake, 4096, Duration::from_secs(10), 4).await;
        let addr = server.addr();
        let response = send_request(addr, "GET", "/events", &service_headers(), b"")
            .expect("stream roundtrip");
        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("text/plain"));
        assert_eq!(response.body_text(), "xxx");

        fake.wait_for_requests(1).await;
        let recorded = fake.recorded_requests();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].header.mode, "serverStream");
        assert_eq!(recorded[0].header.routing.ingress.path, "/events");
        assert_eq!(fake.recorded_cancels().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_chunk_before_start_is_rejected_and_cancelled() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream {
            events: vec![FakeStreamEvent::Chunk {
                seq: 0,
                payload: Bytes::from_static(b"x"),
            }],
        }]);
        let (server, mut fake) = start_stream_server(fake, 4096, Duration::from_secs(10), 4).await;
        let addr = server.addr();
        let response = send_request(addr, "GET", "/events", &service_headers(), b"")
            .expect("protocol rejection roundtrip");
        assert_eq!(response.status, 502);
        let body = serde_json::from_slice::<serde_json::Value>(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "InvalidHttpResponse");
        fake.wait_for_cancels(1).await;
        let cancels = fake.recorded_cancels();
        assert_eq!(cancels.len(), 1);
        assert_eq!(cancels[0].reason, RequestCancelReason::ProtocolError);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_end_before_start_is_rejected_and_cancelled() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream {
            events: vec![FakeStreamEvent::End],
        }]);
        let (server, mut fake) = start_stream_server(fake, 4096, Duration::from_secs(10), 4).await;
        let addr = server.addr();
        let response = send_request(addr, "GET", "/events", &service_headers(), b"")
            .expect("protocol rejection roundtrip");
        assert_eq!(response.status, 502);
        fake.wait_for_cancels(1).await;
        assert_eq!(
            fake.recorded_cancels()[0].reason,
            RequestCancelReason::ProtocolError
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_seq_gap_is_rejected_with_cancel_after_started_response() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream {
            events: vec![
                FakeStreamEvent::Start {
                    status: 200,
                    headers: vec![],
                },
                FakeStreamEvent::Chunk {
                    seq: 0,
                    payload: Bytes::from_static(b"a"),
                },
                FakeStreamEvent::Chunk {
                    seq: 2,
                    payload: Bytes::from_static(b"c"),
                },
            ],
        }]);
        let (server, mut fake) = start_stream_server(fake, 4096, Duration::from_secs(10), 4).await;
        let addr = server.addr();
        let (mut client, head) = StreamClient::connect(addr, "GET", "/events", &service_headers())
            .expect("connect stream");
        assert_eq!(head.status, 200);
        assert_eq!(
            client.read_chunk().expect("first chunk"),
            Some(b"a".to_vec())
        );
        // A protocol error after headers truncates the chunked body: the client
        // may observe a proper terminating frame or a bare EOF. Both are legal
        // terminals; no further data may arrive.
        let mut remainder = Vec::new();
        while let Some(chunk) = read_chunk_tolerate_eof(&mut client) {
            remainder.push(chunk);
        }
        assert!(
            remainder.is_empty(),
            "unexpected trailing chunks: {remainder:?}"
        );
        fake.wait_for_cancels(1).await;
        assert_eq!(
            fake.recorded_cancels()[0].reason,
            RequestCancelReason::ProtocolError
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_cumulative_ceiling_cancels_after_headers() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream {
            events: vec![
                FakeStreamEvent::Start {
                    status: 200,
                    headers: vec![],
                },
                FakeStreamEvent::Chunk {
                    seq: 0,
                    payload: Bytes::from_static(b"aaaaaa"),
                },
                FakeStreamEvent::Chunk {
                    seq: 1,
                    payload: Bytes::from_static(b"bbbbbb"),
                },
            ],
        }]);
        let (server, mut fake) = start_stream_server(fake, 10, Duration::from_secs(10), 4).await;
        let addr = server.addr();
        let (mut client, head) = StreamClient::connect(addr, "GET", "/events", &service_headers())
            .expect("connect stream");
        assert_eq!(head.status, 200);
        assert_eq!(
            client.read_chunk().expect("first chunk"),
            Some(b"aaaaaa".to_vec())
        );
        let mut remainder = Vec::new();
        while let Some(chunk) = read_chunk_tolerate_eof(&mut client) {
            remainder.push(chunk);
        }
        assert!(
            remainder.is_empty(),
            "unexpected trailing chunks: {remainder:?}"
        );
        fake.wait_for_cancels(1).await;
        assert_eq!(
            fake.recorded_cancels()[0].reason,
            RequestCancelReason::ProtocolError
        );
    }

    fn read_chunk_tolerate_eof(client: &mut StreamClient) -> Option<Vec<u8>> {
        match client.read_chunk() {
            Ok(Some(chunk)) => Some(chunk),
            Ok(None) | Err(_) => None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_backpressure_cancels_after_drain_timeout() {
        // The client stops reading after the head; the bounded channel fills and
        // the sink's drain deadline fires `backpressure`. Chunks are large enough
        // to fill the socket send buffer so hyper stops draining the channel.
        let mut events = vec![FakeStreamEvent::Start {
            status: 200,
            headers: vec![],
        }];
        events.extend(stream_chunks_size(256, 64 * 1024));
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream { events }]);
        let (server, mut fake) =
            start_stream_server(fake, 1 << 25, Duration::from_millis(200), 1).await;
        let addr = server.addr();
        let (client, head) = StreamClient::connect(addr, "GET", "/events", &service_headers())
            .expect("connect stream");
        assert_eq!(head.status, 200);
        // Do not read the body; wait for the writer's drain deadline.
        fake.wait_for_cancels(1).await;
        let cancels = fake.recorded_cancels();
        assert_eq!(cancels.len(), 1);
        assert_eq!(cancels[0].reason, RequestCancelReason::Backpressure);
        drop(client);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_runtime_cancel_before_start_is_provider_unavailable() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream {
            events: vec![FakeStreamEvent::RuntimeCancel],
        }]);
        let (server, fake) = start_stream_server(fake, 4096, Duration::from_secs(10), 4).await;
        let addr = server.addr();
        let response = send_request(addr, "GET", "/events", &service_headers(), b"")
            .expect("runtime cancel roundtrip");
        assert_eq!(response.status, 503);
        // Runtime-initiated cancel sends no Router→Runtime cancel frame.
        assert_eq!(fake.recorded_cancels().len(), 0);
    }
}
