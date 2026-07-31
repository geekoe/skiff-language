use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use skiff_artifact_model::{PackageSchemaTypeId, PackageSchemaTypeRecord};

use crate::{LinkedNamedUnionBranch, TypeAddr};

/// Exact Package-owned public identity used by the assembly service-error table.
///
/// This is deliberately a linked-image DTO, not runtime-model `CatchIdentity`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceErrorPublicIdentity {
    package_id: String,
    stable_schema_key: String,
    package_schema_type_id: PackageSchemaTypeId,
}

impl ServiceErrorPublicIdentity {
    pub fn new(
        package_id: impl Into<String>,
        stable_schema_key: impl Into<String>,
        package_schema_type_id: PackageSchemaTypeId,
    ) -> Self {
        Self {
            package_id: package_id.into(),
            stable_schema_key: stable_schema_key.into(),
            package_schema_type_id,
        }
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn stable_schema_key(&self) -> &str {
        &self.stable_schema_key
    }

    pub fn package_schema_type_id(&self) -> &PackageSchemaTypeId {
        &self.package_schema_type_id
    }
}

/// Exact execution lookup key. Named-union branches retain their enclosing
/// declaration address and canonical branch ordinal instead of being inferred
/// from value shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ServiceErrorExecutionKey {
    Declaration {
        addr: TypeAddr,
    },
    NamedUnionBranch {
        union_addr: TypeAddr,
        branch_index: usize,
    },
}

impl ServiceErrorExecutionKey {
    pub fn execution_addr(&self) -> &TypeAddr {
        match self {
            Self::Declaration { addr } => addr,
            Self::NamedUnionBranch { union_addr, .. } => union_addr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorDeclarationKind {
    Record,
    Representation,
}

/// Linked declaration facts retained for later catch-identity materialization.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceErrorExecutionContext {
    Declaration {
        addr: TypeAddr,
        kind: ServiceErrorDeclarationKind,
    },
    NamedUnionBranch {
        union_addr: TypeAddr,
        branch_index: usize,
        branch: LinkedNamedUnionBranch,
        representation_owner: Option<TypeAddr>,
    },
}

impl ServiceErrorExecutionContext {
    pub fn execution_key(&self) -> ServiceErrorExecutionKey {
        match self {
            Self::Declaration { addr, .. } => {
                ServiceErrorExecutionKey::Declaration { addr: addr.clone() }
            }
            Self::NamedUnionBranch {
                union_addr,
                branch_index,
                ..
            } => ServiceErrorExecutionKey::NamedUnionBranch {
                union_addr: union_addr.clone(),
                branch_index: *branch_index,
            },
        }
    }

    pub fn execution_addr(&self) -> &TypeAddr {
        match self {
            Self::Declaration { addr, .. } => addr,
            Self::NamedUnionBranch { union_addr, .. } => union_addr,
        }
    }
}

/// One immutable bidirectional-table row. The canonical Package schema record
/// is the later codec input; no encoded exception or request state is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceErrorTypeLink {
    public_identity: ServiceErrorPublicIdentity,
    record: Arc<PackageSchemaTypeRecord>,
    context: ServiceErrorExecutionContext,
}

impl ServiceErrorTypeLink {
    pub fn try_new(
        public_identity: ServiceErrorPublicIdentity,
        record: Arc<PackageSchemaTypeRecord>,
        context: ServiceErrorExecutionContext,
    ) -> Result<Self, ServiceErrorTypeIndexError> {
        if public_identity.package_id.trim().is_empty()
            || public_identity.package_id.trim() != public_identity.package_id
            || public_identity.stable_schema_key.trim().is_empty()
            || public_identity.stable_schema_key.trim() != public_identity.stable_schema_key
            || public_identity
                .package_schema_type_id
                .as_str()
                .trim()
                .is_empty()
        {
            return Err(ServiceErrorTypeIndexError::InvalidPublicIdentity { public_identity });
        }
        if record.package_id != public_identity.package_id
            || record.stable_schema_key != public_identity.stable_schema_key
            || record.package_schema_type_id != public_identity.package_schema_type_id
        {
            return Err(ServiceErrorTypeIndexError::RecordIdentityMismatch {
                public_identity,
                record_type_id: record.package_schema_type_id.clone(),
            });
        }
        Ok(Self {
            public_identity,
            record,
            context,
        })
    }

    pub fn public_identity(&self) -> &ServiceErrorPublicIdentity {
        &self.public_identity
    }

    pub fn record(&self) -> &Arc<PackageSchemaTypeRecord> {
        &self.record
    }

    pub fn context(&self) -> &ServiceErrorExecutionContext {
        &self.context
    }

    pub fn execution_key(&self) -> ServiceErrorExecutionKey {
        self.context.execution_key()
    }
}

/// Assembly-owned immutable service-error identity index.
#[derive(Debug, Default)]
pub struct ServiceErrorTypeIndex {
    by_execution: HashMap<ServiceErrorExecutionKey, Arc<ServiceErrorTypeLink>>,
    by_public: BTreeMap<ServiceErrorPublicIdentity, Vec<Arc<ServiceErrorTypeLink>>>,
}

impl ServiceErrorTypeIndex {
    pub fn try_new(
        links: impl IntoIterator<Item = ServiceErrorTypeLink>,
    ) -> Result<Self, ServiceErrorTypeIndexError> {
        let mut by_execution = HashMap::new();
        let mut by_public =
            BTreeMap::<ServiceErrorPublicIdentity, Vec<Arc<ServiceErrorTypeLink>>>::new();
        let mut identity_by_addr = HashMap::<TypeAddr, ServiceErrorPublicIdentity>::new();
        let mut record_by_type_id = BTreeMap::<
            PackageSchemaTypeId,
            (ServiceErrorPublicIdentity, Arc<PackageSchemaTypeRecord>),
        >::new();

        for link in links {
            let key = link.execution_key();
            let identity = link.public_identity.clone();
            let addr = link.context.execution_addr().clone();
            if let Some(existing) = identity_by_addr.insert(addr.clone(), identity.clone()) {
                if existing != identity {
                    return Err(
                        ServiceErrorTypeIndexError::ExecutionAddressIdentityConflict {
                            addr,
                            first: existing,
                            second: identity,
                        },
                    );
                }
            }
            if let Some((existing_identity, existing_record)) =
                record_by_type_id.get(&link.record.package_schema_type_id)
            {
                if existing_identity != &identity
                    || existing_record.as_ref() != link.record.as_ref()
                {
                    return Err(ServiceErrorTypeIndexError::TypeRecordConflict {
                        type_id: link.record.package_schema_type_id.clone(),
                    });
                }
            } else {
                record_by_type_id.insert(
                    link.record.package_schema_type_id.clone(),
                    (identity.clone(), Arc::clone(&link.record)),
                );
            }

            let link = Arc::new(link);
            if by_execution
                .insert(key.clone(), Arc::clone(&link))
                .is_some()
            {
                return Err(ServiceErrorTypeIndexError::DuplicateExecutionKey { key });
            }
            by_public.entry(identity).or_default().push(link);
        }

        Ok(Self {
            by_execution,
            by_public,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.by_execution.is_empty()
    }

    pub fn execution_len(&self) -> usize {
        self.by_execution.len()
    }

    pub fn public_identity_len(&self) -> usize {
        self.by_public.len()
    }

    pub fn by_execution(
        &self,
        key: &ServiceErrorExecutionKey,
    ) -> Option<&Arc<ServiceErrorTypeLink>> {
        self.by_execution.get(key)
    }

    pub fn by_public_identity(
        &self,
        identity: &ServiceErrorPublicIdentity,
    ) -> Option<&[Arc<ServiceErrorTypeLink>]> {
        self.by_public.get(identity).map(Vec::as_slice)
    }

    pub fn execution_addresses_by_public_identity(
        &self,
        identity: &ServiceErrorPublicIdentity,
    ) -> Option<Vec<TypeAddr>> {
        self.by_public.get(identity).map(|links| {
            let mut addresses = Vec::new();
            for link in links {
                let addr = link.context.execution_addr();
                if !addresses.contains(addr) {
                    addresses.push(addr.clone());
                }
            }
            addresses
        })
    }

    pub fn public_identities(&self) -> impl ExactSizeIterator<Item = &ServiceErrorPublicIdentity> {
        self.by_public.keys()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceErrorTypeIndexError {
    InvalidPublicIdentity {
        public_identity: ServiceErrorPublicIdentity,
    },
    RecordIdentityMismatch {
        public_identity: ServiceErrorPublicIdentity,
        record_type_id: PackageSchemaTypeId,
    },
    DuplicateExecutionKey {
        key: ServiceErrorExecutionKey,
    },
    ExecutionAddressIdentityConflict {
        addr: TypeAddr,
        first: ServiceErrorPublicIdentity,
        second: ServiceErrorPublicIdentity,
    },
    TypeRecordConflict {
        type_id: PackageSchemaTypeId,
    },
}

impl std::fmt::Display for ServiceErrorTypeIndexError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "service error type index validation failed: {self:?}"
        )
    }
}

impl std::error::Error for ServiceErrorTypeIndexError {}

#[cfg(test)]
mod tests;
