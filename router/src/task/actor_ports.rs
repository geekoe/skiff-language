//! Narrow actor-lane port consumed by the task admission seam (E2b):
//! owner candidate selection, session resolution and outbound frame writes.
//! The production adapter wraps `SessionHandle` + `WsSessionWriter`; tests
//! inject fakes without constructing a full session layer.

use std::fmt;
use std::sync::Arc;

use crate::session::identity::RuntimeSessionEpoch;
use crate::supervisor::session_ports::SessionHandle;
use crate::supervisor::ws::WsSessionWriter;

pub trait TaskActorOwnerPort: Send + Sync + fmt::Debug {
    /// Routable runtime candidates for one deployment build id (sorted,
    /// current, non-cancelled — same projection the ordinary actor lane
    /// uses; M4: build-id keyed).
    fn candidates_by_build_id(&self, build_id: &str) -> Vec<RuntimeSessionEpoch>;

    /// Current registered session for one runtime replica.
    fn current_session_by_replica(&self, replica_id: &str) -> Option<RuntimeSessionEpoch>;

    /// Writes one frame to an exact runtime session (bounded writer queue).
    fn write(&self, session: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct SessionTaskActorOwnerPort {
    session: SessionHandle,
    writer: Arc<dyn WsSessionWriter>,
}

impl SessionTaskActorOwnerPort {
    pub fn new(session: SessionHandle, writer: Arc<dyn WsSessionWriter>) -> Self {
        Self { session, writer }
    }
}

impl TaskActorOwnerPort for SessionTaskActorOwnerPort {
    fn candidates_by_build_id(&self, build_id: &str) -> Vec<RuntimeSessionEpoch> {
        let Some(layer) = self.session.layer() else {
            return Vec::new();
        };
        layer.candidates_by_build_id(build_id)
    }

    fn current_session_by_replica(&self, replica_id: &str) -> Option<RuntimeSessionEpoch> {
        let layer = self.session.layer()?;
        layer.current_session_by_replica(replica_id)
    }

    fn write(&self, session: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
        self.writer.write(session, bytes)
    }
}
