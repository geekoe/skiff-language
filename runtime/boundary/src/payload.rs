use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PayloadBoundary {
    kind: PayloadBoundaryKind,
    trust: PayloadTrust,
    origin_service: Option<PayloadServiceRef>,
    target_service: Option<PayloadServiceRef>,
}

impl PayloadBoundary {
    pub fn owner_internal(kind: PayloadBoundaryKind) -> Self {
        Self::new(kind, PayloadTrust::OwnerInternal)
    }

    pub fn runtime_internal() -> Self {
        Self::owner_internal(PayloadBoundaryKind::RuntimeInternal)
    }

    pub fn cross_service(
        kind: PayloadBoundaryKind,
        target_service: impl Into<PayloadServiceRef>,
    ) -> Self {
        Self::new(kind, PayloadTrust::CrossService).with_target_service(target_service)
    }

    pub fn external_untrusted(kind: PayloadBoundaryKind) -> Self {
        Self::new(kind, PayloadTrust::ExternalUntrusted)
    }

    fn new(kind: PayloadBoundaryKind, trust: PayloadTrust) -> Self {
        Self {
            kind,
            trust,
            origin_service: None,
            target_service: None,
        }
    }

    pub fn with_origin_service(mut self, service: impl Into<PayloadServiceRef>) -> Self {
        self.origin_service = Some(service.into());
        self
    }

    pub fn with_target_service(mut self, service: impl Into<PayloadServiceRef>) -> Self {
        self.target_service = Some(service.into());
        self.trust = PayloadTrust::for_kind_and_target(self.kind, self.target_service.as_ref());
        self
    }

    pub fn kind(&self) -> PayloadBoundaryKind {
        self.kind
    }

    pub fn trust(&self) -> PayloadTrust {
        self.trust
    }

    pub fn origin_service(&self) -> Option<&PayloadServiceRef> {
        self.origin_service.as_ref()
    }

    pub fn target_service(&self) -> Option<&PayloadServiceRef> {
        self.target_service.as_ref()
    }

    pub fn diagnostic_label(&self) -> String {
        let mut parts = vec![
            format!("kind={}", self.kind),
            format!("trust={}", self.trust),
        ];
        if let Some(origin) = &self.origin_service {
            parts.push(format!("origin={origin}"));
        }
        if let Some(target) = &self.target_service {
            parts.push(format!("target={target}"));
        }
        parts.join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PayloadServiceRef {
    service_id: String,
    version: Option<String>,
    build_id: Option<String>,
}

impl PayloadServiceRef {
    pub fn new(service_id: impl Into<String>) -> Self {
        Self {
            service_id: service_id.into(),
            version: None,
            build_id: None,
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_build_id(mut self, build_id: impl Into<String>) -> Self {
        self.build_id = Some(build_id.into());
        self
    }

    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn build_id(&self) -> Option<&str> {
        self.build_id.as_deref()
    }
}

impl From<&str> for PayloadServiceRef {
    fn from(service_id: &str) -> Self {
        Self::new(service_id)
    }
}

impl From<String> for PayloadServiceRef {
    fn from(service_id: String) -> Self {
        Self::new(service_id)
    }
}

impl fmt::Display for PayloadServiceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.service_id)?;
        if let Some(version) = &self.version {
            write!(formatter, "@{version}")?;
        }
        if let Some(build_id) = &self.build_id {
            write!(formatter, "#{build_id}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadBoundaryKind {
    SpawnPayload,
    QueueWorkItemPayload,
    RuntimeWirePayload,
    OutboundServiceCall,
    InboundServiceCall,
    ServiceResponse,
    PublicApiPayload,
    MaterializationPayload,
    WebsocketRequest,
    StreamItem,
    RuntimeInternal,
}

impl fmt::Display for PayloadBoundaryKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SpawnPayload => "SpawnPayload",
            Self::QueueWorkItemPayload => "QueueWorkItemPayload",
            Self::RuntimeWirePayload => "RuntimeWirePayload",
            Self::OutboundServiceCall => "OutboundServiceCall",
            Self::InboundServiceCall => "InboundServiceCall",
            Self::ServiceResponse => "ServiceResponse",
            Self::PublicApiPayload => "PublicApiPayload",
            Self::MaterializationPayload => "MaterializationPayload",
            Self::WebsocketRequest => "WebsocketRequest",
            Self::StreamItem => "StreamItem",
            Self::RuntimeInternal => "RuntimeInternal",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadTrust {
    OwnerInternal,
    CrossService,
    ExternalUntrusted,
}

impl PayloadTrust {
    pub fn for_kind_and_target(
        kind: PayloadBoundaryKind,
        target_service: Option<&PayloadServiceRef>,
    ) -> Self {
        match kind {
            PayloadBoundaryKind::SpawnPayload
            | PayloadBoundaryKind::QueueWorkItemPayload
            | PayloadBoundaryKind::RuntimeInternal => Self::OwnerInternal,
            PayloadBoundaryKind::RuntimeWirePayload if target_service.is_some() => {
                Self::CrossService
            }
            PayloadBoundaryKind::RuntimeWirePayload => Self::OwnerInternal,
            PayloadBoundaryKind::OutboundServiceCall | PayloadBoundaryKind::InboundServiceCall
                if target_service.is_some() =>
            {
                Self::CrossService
            }
            PayloadBoundaryKind::ServiceResponse
            | PayloadBoundaryKind::PublicApiPayload
            | PayloadBoundaryKind::MaterializationPayload => Self::ExternalUntrusted,
            PayloadBoundaryKind::WebsocketRequest | PayloadBoundaryKind::StreamItem => {
                Self::ExternalUntrusted
            }
            PayloadBoundaryKind::OutboundServiceCall | PayloadBoundaryKind::InboundServiceCall => {
                Self::OwnerInternal
            }
        }
    }
}

impl fmt::Display for PayloadTrust {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OwnerInternal => "OwnerInternal",
            Self::CrossService => "CrossService",
            Self::ExternalUntrusted => "ExternalUntrusted",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_trust_is_derived_from_kind_and_target_service() {
        assert_eq!(
            PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload).trust(),
            PayloadTrust::OwnerInternal
        );
        assert_eq!(
            PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload).trust(),
            PayloadTrust::OwnerInternal
        );
        assert_eq!(
            PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
                .with_target_service(PayloadServiceRef::new("skiff.run/account"))
                .trust(),
            PayloadTrust::CrossService
        );
        assert_eq!(
            PayloadBoundary::cross_service(
                PayloadBoundaryKind::OutboundServiceCall,
                PayloadServiceRef::new("skiff.run/account")
            )
            .trust(),
            PayloadTrust::CrossService
        );
        assert_eq!(
            PayloadBoundary::external_untrusted(PayloadBoundaryKind::ServiceResponse).trust(),
            PayloadTrust::ExternalUntrusted
        );
    }

    #[test]
    fn payload_boundary_diagnostic_label_contains_kind_trust_and_services() {
        let boundary = PayloadBoundary::cross_service(
            PayloadBoundaryKind::OutboundServiceCall,
            PayloadServiceRef::new("skiff.run/registry").with_version("0.1.0"),
        )
        .with_origin_service(PayloadServiceRef::new("skiff.run/account").with_build_id("build-1"));

        let label = boundary.diagnostic_label();
        assert!(label.contains("kind=OutboundServiceCall"));
        assert!(label.contains("trust=CrossService"));
        assert!(label.contains("origin=skiff.run/account#build-1"));
        assert!(label.contains("target=skiff.run/registry@0.1.0"));
    }
}
