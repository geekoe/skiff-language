use super::*;

#[derive(Clone)]
pub struct RuntimeOwnedWebsocketParts {
    pub(super) service_id: String,
    pub(super) websocket_entry_id: Option<String>,
    pub(super) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
}

#[derive(Clone)]
pub(super) struct RuntimeWebsocketCapabilityContext<'a> {
    pub(super) context: concrete::WebsocketCapabilityContext<'a>,
    pub(super) owned: RuntimeOwnedWebsocketParts,
}

impl capability_contract::WebsocketCapabilityApi for RuntimeWebsocketCapabilityContext<'_> {
    fn owned(&self) -> capability_contract::OwnedWebsocketCapabilityContext {
        capability_contract::WebsocketCapabilityContext::new(RuntimeOwnedWebsocketCapabilityContext(
            self.owned.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::WebsocketCapabilityContext<'_> {
        websocket(
            concrete::WebsocketCapabilityContext::with_entry_id(
                self.context.service_id(),
                self.context.websocket_entry_id(),
                self.owned.router_sender.as_ref(),
            ),
            self.owned.clone(),
        )
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
            .map_err(capability_contract::CapabilityError::opaque)
    }

    fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> capability_contract::CapabilityResult<()> {
        self.context
            .send_connection_binary_to_business_identity(business_identity, payload)
            .map_err(capability_contract::CapabilityError::opaque)
    }

    fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> capability_contract::CapabilityResult<()> {
        self.context
            .send_connection_text_to_connection(connection_id, text)
            .map_err(capability_contract::CapabilityError::opaque)
    }

    fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> capability_contract::CapabilityResult<()> {
        self.context
            .send_connection_binary_to_connection(connection_id, payload)
            .map_err(capability_contract::CapabilityError::opaque)
    }
}

struct RuntimeOwnedWebsocketCapabilityContext(RuntimeOwnedWebsocketParts);

impl capability_contract::WebsocketCapabilityApi for RuntimeOwnedWebsocketCapabilityContext {
    fn owned(&self) -> capability_contract::OwnedWebsocketCapabilityContext {
        capability_contract::WebsocketCapabilityContext::new(RuntimeOwnedWebsocketCapabilityContext(
            self.0.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::WebsocketCapabilityContext<'_> {
        websocket_from_request(
            &self.0.service_id,
            self.0.websocket_entry_id.as_deref(),
            self.0.router_sender.as_ref(),
        )
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
