use std::collections::HashMap;

use crate::{OutboundRequestLease, OutboundResponseReceiver, RequestEffectDoubleControl};

pub struct OutboundServiceRequestStart {
    pub mode: String,
    pub target: String,
    pub operation_abi_id: String,
    pub selector: String,
    pub service_id: String,
    pub version: String,
    pub build_id: String,
    pub service_protocol_identity: String,
    pub activation_identity: Option<String>,
    pub timeout_ms: Option<u64>,
    pub test_effect_doubles: HashMap<String, Vec<RequestEffectDoubleControl>>,
}

pub struct OutboundStartedRequest {
    pub request_id: String,
    pub response_rx: OutboundResponseReceiver,
    pub lease: OutboundRequestLease,
}
