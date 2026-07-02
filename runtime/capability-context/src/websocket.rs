use std::sync::Arc;

use crate::CapabilityResult;

pub trait WebsocketCapabilityApi: Send + Sync {
    fn owned(&self) -> OwnedWebsocketCapabilityContext;
    fn borrow(&self) -> WebsocketCapabilityContext<'_>;
    fn service_id(&self) -> &str;
    fn websocket_entry_id(&self) -> Option<&str>;
    fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> CapabilityResult<()>;
    fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> CapabilityResult<()>;
    fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> CapabilityResult<()>;
    fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> CapabilityResult<()>;
}

#[derive(Clone)]
pub struct WebsocketCapabilityContext<'a> {
    inner: Arc<dyn WebsocketCapabilityApi + 'a>,
}

impl<'a> WebsocketCapabilityContext<'a> {
    pub fn new<T>(inner: T) -> Self
    where
        T: WebsocketCapabilityApi + 'a,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn owned(&self) -> OwnedWebsocketCapabilityContext {
        self.inner.owned()
    }

    pub fn borrow(&self) -> WebsocketCapabilityContext<'_> {
        self.inner.borrow()
    }

    pub fn service_id(&self) -> &str {
        self.inner.service_id()
    }

    pub fn websocket_entry_id(&self) -> Option<&str> {
        self.inner.websocket_entry_id()
    }

    pub fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> CapabilityResult<()> {
        self.inner
            .send_connection_text_to_business_identity(business_identity, text)
    }

    pub fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> CapabilityResult<()> {
        self.inner
            .send_connection_binary_to_business_identity(business_identity, payload)
    }

    pub fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> CapabilityResult<()> {
        self.inner
            .send_connection_text_to_connection(connection_id, text)
    }

    pub fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> CapabilityResult<()> {
        self.inner
            .send_connection_binary_to_connection(connection_id, payload)
    }
}

pub type OwnedWebsocketCapabilityContext = WebsocketCapabilityContext<'static>;
