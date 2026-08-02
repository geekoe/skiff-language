//! W-http real-socket probes: deadline timeout and client-disconnect cancel
//! terminals through the fake dispatcher.

mod http_common;

#[cfg(test)]
mod tests {

    use std::sync::Arc;
    use std::time::Duration;

    use bytes::Bytes;
    use skiff_router::http::fake::{FakeDispatchPlan, FakeHttpDispatcher, FakeStreamEvent};
    use skiff_router::http::{start_http_gateway, HttpGatewayServer, HttpGatewayServerOptions};
    use skiff_runtime_transport::cancel_reason::RequestCancelReason;

    use crate::http_common;
    use crate::http_common::{
        fixture_epoch, fixture_resolver, send_request, service_headers, StreamClient,
    };

    async fn start_server(
        dispatcher: FakeHttpDispatcher,
        request_timeout: Duration,
    ) -> (HttpGatewayServer, FakeHttpDispatcher) {
        let dispatcher = Arc::new(dispatcher);
        let options = HttpGatewayServerOptions {
            request_timeout,
            ..HttpGatewayServerOptions::new("127.0.0.1:0".parse().expect("bind"), 1024 * 1024, 4096)
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

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unary_timeout_returns_504_and_records_timeout_cancel() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryHang]);
        let (server, mut fake) = start_server(fake, Duration::from_millis(200)).await;
        let addr = server.addr();
        let response = send_request(addr, "POST", "/items", &service_headers(), b"{}")
            .expect("timeout roundtrip");
        assert_eq!(response.status, 504);
        let body = serde_json::from_slice::<serde_json::Value>(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "TimeoutError");
        fake.wait_for_cancels(1).await;
        assert_eq!(
            fake.recorded_cancels()[0].reason,
            RequestCancelReason::Timeout
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_stream_timeout_before_start_returns_504() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::StreamHang]);
        let (server, mut fake) = start_server(fake, Duration::from_millis(200)).await;
        let addr = server.addr();
        let response =
            send_request(addr, "GET", "/events", &service_headers(), b"").expect("stream timeout");
        assert_eq!(response.status, 504);
        let body = serde_json::from_slice::<serde_json::Value>(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "TimeoutError");
        fake.wait_for_cancels(1).await;
        assert_eq!(
            fake.recorded_cancels()[0].reason,
            RequestCancelReason::Timeout
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_client_disconnect_mid_stream_records_client_disconnect_cancel() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::Stream {
            events: vec![
                FakeStreamEvent::Start {
                    status: 200,
                    headers: vec![],
                },
                FakeStreamEvent::Chunk {
                    seq: 0,
                    payload: Bytes::from_static(b"first"),
                },
                FakeStreamEvent::Delay {
                    duration: Duration::from_millis(300),
                },
                FakeStreamEvent::Chunk {
                    seq: 1,
                    payload: Bytes::from_static(b"second"),
                },
                FakeStreamEvent::End,
            ],
        }]);
        let (server, mut fake) = start_server(fake, Duration::from_secs(5)).await;
        let addr = server.addr();
        let (mut client, head) = StreamClient::connect(addr, "GET", "/events", &service_headers())
            .expect("connect stream");
        assert_eq!(head.status, 200);
        assert_eq!(
            client.read_chunk().expect("first chunk"),
            Some(b"first".to_vec())
        );
        // Drop the socket while the fake dispatcher is paused mid-stream.
        drop(client);
        fake.wait_for_cancels(1).await;
        assert_eq!(
            fake.recorded_cancels()[0].reason,
            RequestCancelReason::ClientDisconnect
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unary_client_disconnect_mid_dispatch_records_cancel() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryHang]);
        let (server, mut fake) = start_server(fake, Duration::from_secs(5)).await;
        let addr = server.addr();
        // Raw socket: send the request then drop it while dispatch is pending.
        {
            use std::io::Write;
            let mut stream = std::net::TcpStream::connect(addr).expect("connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let request = format!(
            "POST /items HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\nx-skiff-service: {}\r\nx-skiff-version: {}\r\ncontent-length: 2\r\n\r\n{{}}",
            http_common::SERVICE_ID,
            http_common::CONTRACT_VERSION
        );
            stream.write_all(request.as_bytes()).expect("send request");
            // Give the server time to start dispatch, then disconnect.
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        fake.wait_for_cancels(1).await;
        assert_eq!(
            fake.recorded_cancels()[0].reason,
            RequestCancelReason::ClientDisconnect
        );
    }
}
