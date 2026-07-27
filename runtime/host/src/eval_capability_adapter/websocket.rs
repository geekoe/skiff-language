use super::*;

#[derive(Clone)]
pub struct RuntimeOwnedWebsocketParts {
    pub(super) service_id: String,
    pub(super) websocket_entry_id: Option<String>,
    pub(super) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(super) request_transport: Option<RuntimeConnectionRequestParts>,
}

#[derive(Clone)]
pub(super) struct RuntimeConnectionRequestParts {
    pub(super) registry: Arc<ConnectionRequestRegistry>,
    pub(super) session: ConnectionRequestSession,
    pub(super) cancellation: CancellationToken,
    pub(super) deadline: Option<Instant>,
}

#[derive(Clone)]
pub(super) struct RuntimeWebsocketCapabilityContext<'a> {
    pub(super) context: concrete::WebsocketCapabilityContext<'a>,
    pub(super) owned: RuntimeOwnedWebsocketParts,
}

impl capability_contract::WebsocketCapabilityApi for RuntimeWebsocketCapabilityContext<'_> {
    fn owned(&self) -> capability_contract::OwnedWebsocketCapabilityContext {
        capability_contract::WebsocketCapabilityContext::new(
            RuntimeOwnedWebsocketCapabilityContext(self.owned.clone()),
        )
    }

    fn borrow(&self) -> capability_contract::WebsocketCapabilityContext<'_> {
        capability_contract::WebsocketCapabilityContext::new(RuntimeWebsocketCapabilityContext {
            context: concrete::WebsocketCapabilityContext::with_entry_id(
                self.context.service_id(),
                self.context.websocket_entry_id(),
                self.owned.router_sender.as_ref(),
            ),
            owned: self.owned.clone(),
        })
    }

    fn service_id(&self) -> &str {
        self.context.service_id()
    }

    fn websocket_entry_id(&self) -> Option<&str> {
        self.context.websocket_entry_id()
    }

    fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> capability_contract::CapabilityResult<()> {
        self.context
            .send_connection_text_to_business_identity(business_identity, text)
            .map_err(ordinary_root_error_into_capability)
    }

    fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> capability_contract::CapabilityResult<()> {
        self.context
            .send_connection_binary_to_business_identity(business_identity, payload)
            .map_err(ordinary_root_error_into_capability)
    }

    fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> capability_contract::CapabilityResult<()> {
        self.context
            .send_connection_text_to_connection(connection_id, text)
            .map_err(ordinary_root_error_into_capability)
    }

    fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> capability_contract::CapabilityResult<()> {
        self.context
            .send_connection_binary_to_connection(connection_id, payload)
            .map_err(ordinary_root_error_into_capability)
    }
}

struct RuntimeOwnedWebsocketCapabilityContext(RuntimeOwnedWebsocketParts);

pub(super) struct RuntimeWebsocketRequestCapabilityContext(pub(super) RuntimeOwnedWebsocketParts);

impl eval_capabilities::WebsocketRequestCapabilityApi for RuntimeWebsocketRequestCapabilityContext {
    fn request_json_to_connection<'a>(
        &'a self,
        connection_id: String,
        method: String,
        payload: Vec<u8>,
    ) -> eval_capabilities::EvalCapabilityFuture<'a, capability_contract::ConnectionRequestTerminal>
    {
        let owned = self.0.clone();
        Box::pin(async move {
            let transport = owned.request_transport.as_ref().ok_or_else(|| {
                RuntimeError::Unsupported(
                    "std.websocket.requestJsonToConnection execution is not attached".to_string(),
                )
            })?;
            let deadline_control = transport
                .deadline
                .map(connection_request_deadline_control)
                .transpose()?;
            let deadline = transport.deadline.map(tokio::time::Instant::from_std);
            let context = concrete::WebsocketCapabilityContext::with_entry_id(
                &owned.service_id,
                owned.websocket_entry_id.as_deref(),
                owned.router_sender.as_ref(),
            )
            .with_request_transport(
                transport.registry.as_ref(),
                &transport.session,
                &transport.cancellation,
                deadline,
                deadline_control.as_ref(),
            );
            Ok(context
                .request_json_to_connection(connection_id, method, payload)
                .await)
        })
    }
}

fn connection_request_deadline_control(deadline: Instant) -> Result<RuntimeDeadlineControl> {
    const JS_SAFE_INTEGER_MAX: u128 = 9_007_199_254_740_991;

    let remaining = deadline.saturating_duration_since(Instant::now());
    let timeout_ms: u64 = remaining
        .as_millis()
        .max(1)
        .min(JS_SAFE_INTEGER_MAX)
        .try_into()
        .expect("bounded timeout fits u64");
    let expires_at = time::OffsetDateTime::now_utc()
        .checked_add(time::Duration::milliseconds(
            timeout_ms.try_into().map_err(|_| {
                RuntimeError::InvalidArtifact(
                    "WebSocket request deadline is not representable".to_string(),
                )
            })?,
        ))
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "WebSocket request deadline expiry is not representable".to_string(),
            )
        })?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| {
            RuntimeError::InvalidArtifact(format!(
                "WebSocket request deadline cannot be encoded: {error}"
            ))
        })?;
    Ok(RuntimeDeadlineControl {
        timeout_ms,
        expires_at,
    })
}

impl capability_contract::WebsocketCapabilityApi for RuntimeOwnedWebsocketCapabilityContext {
    fn owned(&self) -> capability_contract::OwnedWebsocketCapabilityContext {
        capability_contract::WebsocketCapabilityContext::new(
            RuntimeOwnedWebsocketCapabilityContext(self.0.clone()),
        )
    }

    fn borrow(&self) -> capability_contract::WebsocketCapabilityContext<'_> {
        capability_contract::WebsocketCapabilityContext::new(RuntimeWebsocketCapabilityContext {
            context: concrete::WebsocketCapabilityContext::with_entry_id(
                &self.0.service_id,
                self.0.websocket_entry_id.as_deref(),
                self.0.router_sender.as_ref(),
            ),
            owned: self.0.clone(),
        })
    }

    fn service_id(&self) -> &str {
        &self.0.service_id
    }

    fn websocket_entry_id(&self) -> Option<&str> {
        self.0.websocket_entry_id.as_deref()
    }

    fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> capability_contract::CapabilityResult<()> {
        self.borrow()
            .send_connection_text_to_business_identity(business_identity, text)
    }

    fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> capability_contract::CapabilityResult<()> {
        self.borrow()
            .send_connection_binary_to_business_identity(business_identity, payload)
    }

    fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> capability_contract::CapabilityResult<()> {
        self.borrow()
            .send_connection_text_to_connection(connection_id, text)
    }

    fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> capability_contract::CapabilityResult<()> {
        self.borrow()
            .send_connection_binary_to_connection(connection_id, payload)
    }
}
