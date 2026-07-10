use skiff_runtime_capability_context::flag_backed_cancel_waiters_active;
use skiff_runtime_transport::protocol::{
    encode_binary_frame, RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::mpsc;

use crate::{
    capability_context::stream_runtime_streams_active,
    error::{Result, RuntimeError},
};

use super::{RouterWriterMessage, RuntimeHost};

impl RuntimeHost {
    pub(crate) async fn queue_runtime_health_with_counters(
        &self,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        runtime_id: &str,
        counters: RuntimeHealthCountersFrameHeader,
    ) -> Result<()> {
        let header = self
            .runtime_health_frame_header_with_counters(runtime_id, counters)
            .await?;
        let frame = encode_binary_frame(&header, &[])
            .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        sender
            .send(RouterWriterMessage::Binary(frame))
            .map_err(|_| RuntimeError::Decode("runtime writer channel closed".to_string()))?;
        Ok(())
    }

    async fn runtime_health_frame_header_with_counters(
        &self,
        runtime_id: &str,
        counters: RuntimeHealthCountersFrameHeader,
    ) -> Result<RuntimeHealthFrameHeader> {
        Ok(RuntimeHealthFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.health".to_string(),
            runtime_id: runtime_id.to_string(),
            observed_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|error| RuntimeError::Decode(error.to_string()))?,
            counters,
        })
    }

    pub(crate) async fn runtime_health_counters(&self) -> RuntimeHealthCountersFrameHeader {
        RuntimeHealthCountersFrameHeader {
            outbound_requests_pending: self.outbound_requests.pending_count(),
            outbound_stream_leases_active: self.outbound_requests.active_lease_count(),
            stream_runtime_streams_active: stream_runtime_streams_active(),
            flag_backed_cancel_waiters_active: flag_backed_cancel_waiters_active(),
            spawned_tasks_active: self.request_supervisor.active_count().await,
        }
    }
}
