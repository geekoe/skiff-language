use std::cell::Cell;

use super::*;

#[derive(Default)]
struct TestProjectionSource {
    actor: Cell<usize>,
    file: Cell<usize>,
    time: Cell<usize>,
    http_client: Cell<usize>,
    http_response_stream: Cell<usize>,
    websocket: Cell<usize>,
    telemetry: Cell<usize>,
    resource: Cell<usize>,
}

impl TestProjectionSource {
    fn increment(counter: &Cell<usize>, value: &'static str) -> &'static str {
        counter.set(counter.get() + 1);
        value
    }

    fn call_counts(&self) -> [usize; 8] {
        [
            self.actor.get(),
            self.file.get(),
            self.time.get(),
            self.http_client.get(),
            self.http_response_stream.get(),
            self.websocket.get(),
            self.telemetry.get(),
            self.resource.get(),
        ]
    }
}

impl NativeCapabilityProjectionSource for TestProjectionSource {
    type Actor = &'static str;
    type File = NativeFileCapabilityContext<&'static str, &'static str, &'static str>;
    type Time = &'static str;
    type HttpClient = NativeHttpClientCapabilityContext<&'static str>;
    type HttpResponseStream = NativeHttpResponseStreamCapabilityContext<&'static str>;
    type Websocket = &'static str;
    type Telemetry = NativeTelemetryCapabilityContext<&'static str>;
    type Resource = &'static str;

    fn actor(&self) -> Self::Actor {
        Self::increment(&self.actor, "actor")
    }

    fn file(&self) -> Self::File {
        NativeFileCapabilityContext::new(
            Self::increment(&self.file, "file"),
            "file_source_stream",
            "heap_limits",
        )
    }

    fn time(&self) -> Self::Time {
        Self::increment(&self.time, "time")
    }

    fn http_client(&self) -> Self::HttpClient {
        NativeHttpClientCapabilityContext::new(Self::increment(&self.http_client, "http_client"))
    }

    fn http_response_stream(&self) -> Self::HttpResponseStream {
        NativeHttpResponseStreamCapabilityContext::new(Self::increment(
            &self.http_response_stream,
            "http_response_stream",
        ))
    }

    fn websocket(&self) -> Self::Websocket {
        Self::increment(&self.websocket, "websocket")
    }

    fn telemetry(&self) -> Self::Telemetry {
        NativeTelemetryCapabilityContext::new(Self::increment(&self.telemetry, "telemetry"))
    }

    fn resource(&self) -> Self::Resource {
        Self::increment(&self.resource, "resource")
    }
}

#[test]
fn native_capability_projection_covers_every_required_context_variant() {
    let cases = [
        NativeRequiredContext::None,
        NativeRequiredContext::Actor,
        NativeRequiredContext::File,
        NativeRequiredContext::Time,
        NativeRequiredContext::HttpClient,
        NativeRequiredContext::HttpResponseStream,
        NativeRequiredContext::Websocket,
        NativeRequiredContext::Telemetry,
        NativeRequiredContext::Resource,
    ];

    for required_context in cases {
        let source = TestProjectionSource::default();
        let projected = project_native_capability_context(required_context, &source);

        assert_eq!(projected.required_context(), required_context);
        match (required_context, projected) {
            (NativeRequiredContext::None, NativeCapabilityContexts::None) => {
                assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 0, 0, 0]);
            }
            (NativeRequiredContext::Actor, NativeCapabilityContexts::Actor(value)) => {
                assert_eq!(value, "actor");
                assert_eq!(source.call_counts(), [1, 0, 0, 0, 0, 0, 0, 0]);
            }
            (NativeRequiredContext::File, NativeCapabilityContexts::File(value)) => {
                assert_eq!(
                    value.into_parts(),
                    ("file", "file_source_stream", "heap_limits")
                );
                assert_eq!(source.call_counts(), [0, 1, 0, 0, 0, 0, 0, 0]);
            }
            (NativeRequiredContext::Time, NativeCapabilityContexts::Time(value)) => {
                assert_eq!(value, "time");
                assert_eq!(source.call_counts(), [0, 0, 1, 0, 0, 0, 0, 0]);
            }
            (NativeRequiredContext::HttpClient, NativeCapabilityContexts::HttpClient(value)) => {
                assert_eq!(value.into_effect_context(), "http_client");
                assert_eq!(source.call_counts(), [0, 0, 0, 1, 0, 0, 0, 0]);
            }
            (
                NativeRequiredContext::HttpResponseStream,
                NativeCapabilityContexts::HttpResponseStream(value),
            ) => {
                assert_eq!(value.into_execution_context(), "http_response_stream");
                assert_eq!(source.call_counts(), [0, 0, 0, 0, 1, 0, 0, 0]);
            }
            (NativeRequiredContext::Websocket, NativeCapabilityContexts::Websocket(value)) => {
                assert_eq!(value, "websocket");
                assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 1, 0, 0]);
            }
            (NativeRequiredContext::Telemetry, NativeCapabilityContexts::Telemetry(value)) => {
                assert_eq!(value.into_effect_context(), "telemetry");
                assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 0, 1, 0]);
            }
            (NativeRequiredContext::Resource, NativeCapabilityContexts::Resource(value)) => {
                assert_eq!(value, "resource");
                assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 0, 0, 1]);
            }
            (expected, actual) => panic!(
                "required context {expected:?} projected unexpected variant {:?}",
                actual.required_context()
            ),
        }
    }
}
