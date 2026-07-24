use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity,
    ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec, plan::BoundaryUse, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkedActorDeclaration, LinkedActorDeclarationOwner, LinkedFileUnit,
    UnitAddr,
};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};
use thiserror::Error;

pub const ACTOR_BOOTSTRAP_ENCODING_V1: &str = "skiff-canonical-v1";

/// Canonical registry identity excluding the incarnation epoch.
///
/// `canonical_actor_id_key_bytes` is already decoded from the registry wire
/// field. Keeping the canonical bytes in the key makes equality independent of
/// a diagnostic hash implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorLogicalKey {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes: Vec<u8>,
    pub actor_id_hash: String,
}

impl ActorLogicalKey {
    fn validate(&self) -> Result<(), ActorInstanceStoreError> {
        for (label, value) in [
            ("serviceId", self.service_id.as_str()),
            ("actorTypeIdentity", self.actor_type_identity.as_str()),
            ("actorIdTypeIdentity", self.actor_id_type_identity.as_str()),
            (
                "actorIdEncodingVersion",
                self.actor_id_encoding_version.as_str(),
            ),
            ("actorIdHash", self.actor_id_hash.as_str()),
        ] {
            if value.is_empty() {
                return Err(ActorInstanceStoreError::InvalidLogicalKey {
                    message: format!("{label} must be non-empty"),
                });
            }
        }
        if self.canonical_actor_id_key_bytes.is_empty() {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: "canonical actor id key bytes must be non-empty".to_string(),
            });
        }
        if self.actor_id_encoding_version != ACTOR_BOOTSTRAP_ENCODING_V1 {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: format!(
                    "unsupported actor id encoding {}",
                    self.actor_id_encoding_version
                ),
            });
        }
        let actor_id: Value =
            serde_json::from_slice(&self.canonical_actor_id_key_bytes).map_err(|error| {
                ActorInstanceStoreError::InvalidLogicalKey {
                    message: format!("actor id key bytes are not JSON: {error}"),
                }
            })?;
        let canonical = canonical_json_bytes(&actor_id).map_err(|error| {
            ActorInstanceStoreError::InvalidLogicalKey {
                message: format!("actor id key cannot be canonicalized: {error}"),
            }
        })?;
        if canonical != self.canonical_actor_id_key_bytes {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: "actor id key bytes are not canonical JSON".to_string(),
            });
        }
        let expected_hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(&self.canonical_actor_id_key_bytes))
        );
        if self.actor_id_hash != expected_hash {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: "actor id hash does not match canonical key bytes".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorIncarnationKey {
    pub logical_key: ActorLogicalKey,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorInstanceFence {
    pub incarnation: ActorIncarnationKey,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: LinkedActorDeclarationOwner,
}

impl ActorInstanceFence {
    fn validate(&self) -> Result<(), ActorInstanceStoreError> {
        self.incarnation.logical_key.validate()?;
        if self.incarnation.epoch == 0 {
            return Err(ActorInstanceStoreError::InvalidEpoch);
        }
        Ok(())
    }
}

pub struct ActorActivationRequest<'a> {
    pub fence: ActorInstanceFence,
    pub bootstrap_encoding_version: &'a str,
    pub bootstrap_payload: &'a [u8],
    pub program: ProgramTypeView<'a>,
}

/// Opaque executor-facing identity. It intentionally exposes no field values.
#[derive(Debug, Clone)]
pub struct ActorInstanceHandle {
    fence: ActorInstanceFence,
    instance: Arc<ActorInstance>,
}

impl ActorInstanceHandle {
    pub fn fence(&self) -> &ActorInstanceFence {
        &self.fence
    }
}

#[derive(Debug)]
struct ActorInstance {
    fence: ActorInstanceFence,
    state: Mutex<ActorInstanceState>,
}

#[derive(Debug)]
struct ActorInstanceState {
    fields: Vec<ActorFieldValue>,
    heap: RequestHeap,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActorFieldValue {
    pub name: String,
    pub value: RuntimeValue,
}

/// Capability token reserved for the Actor executor in this crate.
///
/// Host request paths cannot construct this token and an instance handle does
/// not itself grant field access.
pub(crate) struct ActorExecutorAuthority(());

impl ActorExecutorAuthority {
    pub(crate) fn new() -> Self {
        Self(())
    }
}

#[derive(Debug, Default)]
pub struct ActorInstanceStore {
    state: Mutex<ActorInstanceStoreState>,
}

#[derive(Debug, Default)]
struct ActorInstanceStoreState {
    instances: HashMap<ActorIncarnationKey, Arc<ActorInstance>>,
    latest_epochs: HashMap<ActorLogicalKey, u64>,
}

impl ActorInstanceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically reuses or materializes one exact incarnation.
    ///
    /// Decoding runs while the store mutex is held. Activation is intentionally
    /// uncommon, and this makes publication atomic: concurrent callers can
    /// never observe or duplicate a partially initialized instance.
    pub fn activate(
        &self,
        request: ActorActivationRequest<'_>,
    ) -> Result<ActorInstanceHandle, ActorInstanceStoreError> {
        request.fence.validate()?;
        let mut state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");

        if let Some(latest_epoch) = state
            .latest_epochs
            .get(&request.fence.incarnation.logical_key)
            .copied()
        {
            if request.fence.incarnation.epoch < latest_epoch {
                return Err(ActorInstanceStoreError::StaleEpoch {
                    requested: request.fence.incarnation.epoch,
                    latest: latest_epoch,
                });
            }
        }

        if let Some(existing) = state.instances.get(&request.fence.incarnation) {
            ensure_instance_fence(existing, &request.fence)?;
            return Ok(ActorInstanceHandle {
                fence: request.fence,
                instance: Arc::clone(existing),
            });
        }

        let declaration =
            resolve_actor_declaration(request.program, &request.fence.declaration_owner)?;
        validate_declaration_fence(declaration, &request.fence)?;
        let instance = Arc::new(materialize_instance(&request, declaration)?);

        state
            .latest_epochs
            .entry(request.fence.incarnation.logical_key.clone())
            .and_modify(|epoch| *epoch = (*epoch).max(request.fence.incarnation.epoch))
            .or_insert(request.fence.incarnation.epoch);
        state
            .instances
            .insert(request.fence.incarnation.clone(), Arc::clone(&instance));

        Ok(ActorInstanceHandle {
            fence: request.fence,
            instance,
        })
    }

    /// Removes only the exact materialized instance represented by `handle`.
    ///
    /// Pointer identity closes the same-epoch cleanup race: an old cleanup
    /// handle cannot remove a later re-materialization with otherwise identical
    /// logical and declaration fences.
    pub fn discard_exact(&self, handle: &ActorInstanceHandle) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");
        let matches = state
            .instances
            .get(&handle.fence.incarnation)
            .is_some_and(|instance| {
                ensure_instance_fence(instance, &handle.fence).is_ok()
                    && Arc::ptr_eq(instance, &handle.instance)
            });
        if matches {
            state.instances.remove(&handle.fence.incarnation);
        }
        matches
    }

    pub fn len(&self) -> usize {
        self.state
            .lock()
            .expect("actor instance store lock poisoned")
            .instances
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The later Actor executor enters field state through this exact-fence
    /// gate. The method is crate-private so ordinary host/request consumers
    /// cannot turn an opaque handle into field access.
    pub(crate) fn with_fields_for_executor<T>(
        &self,
        _authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
        operation: impl FnOnce(&mut [ActorFieldValue], &mut RequestHeap) -> T,
    ) -> Result<T, ActorInstanceStoreError> {
        let instance = {
            let state = self
                .state
                .lock()
                .expect("actor instance store lock poisoned");
            let current = state
                .instances
                .get(&handle.fence.incarnation)
                .ok_or(ActorInstanceStoreError::InstanceNotFound)?;
            if state
                .latest_epochs
                .get(&handle.fence.incarnation.logical_key)
                .is_some_and(|latest| *latest != handle.fence.incarnation.epoch)
            {
                return Err(ActorInstanceStoreError::StaleEpoch {
                    requested: handle.fence.incarnation.epoch,
                    latest: state.latest_epochs[&handle.fence.incarnation.logical_key],
                });
            }
            ensure_instance_fence(current, &handle.fence)?;
            if !Arc::ptr_eq(current, &handle.instance) {
                return Err(ActorInstanceStoreError::InstanceReplaced);
            }
            Arc::clone(current)
        };
        let mut state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        let ActorInstanceState { fields, heap } = &mut *state;
        Ok(operation(fields, heap))
    }
}

fn ensure_instance_fence(
    instance: &ActorInstance,
    requested: &ActorInstanceFence,
) -> Result<(), ActorInstanceStoreError> {
    if instance.fence == *requested {
        Ok(())
    } else {
        Err(ActorInstanceStoreError::FenceMismatch)
    }
}

fn materialize_instance(
    request: &ActorActivationRequest<'_>,
    declaration: &LinkedActorDeclaration,
) -> Result<ActorInstance, ActorInstanceStoreError> {
    if request.bootstrap_encoding_version != ACTOR_BOOTSTRAP_ENCODING_V1 {
        return Err(ActorInstanceStoreError::UnsupportedBootstrapEncoding {
            actual: request.bootstrap_encoding_version.to_string(),
        });
    }
    let payload: Value = serde_json::from_slice(request.bootstrap_payload).map_err(|error| {
        ActorInstanceStoreError::BootstrapDecode {
            message: error.to_string(),
        }
    })?;
    let object = payload
        .as_object()
        .ok_or(ActorInstanceStoreError::BootstrapNotRecord)?;

    let mut canonical_field_names = declaration
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    canonical_field_names.sort_unstable();
    let actual_names = object.keys().map(String::as_str).collect::<Vec<_>>();
    if actual_names != canonical_field_names {
        return Err(ActorInstanceStoreError::BootstrapFieldShape {
            expected: canonical_field_names
                .into_iter()
                .map(str::to_string)
                .collect(),
            actual: actual_names.into_iter().map(str::to_string).collect(),
        });
    }

    let current_addr = ExecutableAddr {
        unit: request.fence.declaration_owner.unit.clone(),
        file: request.fence.declaration_owner.file.clone(),
        executable: 0,
    };
    let context = PlanContext::from_type_view(request.program, &current_addr);
    let mut heap = RequestHeap::default();
    let mut fields = Vec::with_capacity(declaration.fields.len());
    for field in &declaration.fields {
        if field.encoding != ActorFieldEncodingIr::CanonicalValueV1 {
            return Err(ActorInstanceStoreError::UnsupportedFieldEncoding {
                field: field.name.clone(),
            });
        }
        let plan = RuntimeTypePlan::from_linked(&field.ty, &context).map_err(|error| {
            ActorInstanceStoreError::DeclarationType {
                field: field.name.clone(),
                message: error.to_string(),
            }
        })?;
        let value = RuntimeBoundaryCodec::new(
            &plan,
            BoundaryUse::NativeArg,
            format!("Actor bootstrap field {}", field.name),
        )
        .from_wire_json(
            object
                .get(&field.name)
                .expect("exact bootstrap field shape checked"),
            &mut heap,
        )
        .map_err(|error| ActorInstanceStoreError::BootstrapFieldDecode {
            field: field.name.clone(),
            message: error.to_string(),
        })?;
        fields.push(ActorFieldValue {
            name: field.name.clone(),
            value,
        });
    }
    Ok(ActorInstance {
        fence: request.fence.clone(),
        state: Mutex::new(ActorInstanceState { fields, heap }),
    })
}

fn validate_declaration_fence(
    declaration: &LinkedActorDeclaration,
    fence: &ActorInstanceFence,
) -> Result<(), ActorInstanceStoreError> {
    if declaration.actor_runtime_abi_version != ACTOR_RUNTIME_ABI_VERSION_V1 {
        return Err(ActorInstanceStoreError::UnsupportedActorRuntimeAbi {
            actual: declaration.actor_runtime_abi_version.clone(),
        });
    }
    if declaration.implementation_owner.as_ref() != Some(&fence.declaration_owner) {
        return Err(ActorInstanceStoreError::DeclarationOwnerMismatch);
    }
    if declaration.actor_abi_identity != fence.actor_abi_identity {
        return Err(ActorInstanceStoreError::ActorAbiMismatch);
    }
    if declaration.actor_implementation_identity != fence.actor_implementation_identity {
        return Err(ActorInstanceStoreError::ActorImplementationMismatch);
    }
    Ok(())
}

fn resolve_actor_declaration<'a>(
    program: ProgramTypeView<'a>,
    owner: &LinkedActorDeclarationOwner,
) -> Result<&'a LinkedActorDeclaration, ActorInstanceStoreError> {
    let files = match owner.unit {
        UnitAddr::Service => program.service_files,
        UnitAddr::Package(slot) => program
            .package_files
            .get(slot)
            .map(Vec::as_slice)
            .ok_or(ActorInstanceStoreError::DeclarationFileMissing)?,
    };
    let file = match &owner.file {
        FileAddr::LoadedFileIndex(index) => files.get(*index),
        FileAddr::FileIrIdentity(identity) => {
            files.iter().find(|file| file.file_ir_identity == *identity)
        }
    }
    .ok_or(ActorInstanceStoreError::DeclarationFileMissing)?;
    exact_declaration_in_file(file, owner)
}

fn exact_declaration_in_file<'a>(
    file: &'a Arc<LinkedFileUnit>,
    owner: &LinkedActorDeclarationOwner,
) -> Result<&'a LinkedActorDeclaration, ActorInstanceStoreError> {
    let mut matches = file.actor_declarations.iter().filter(|declaration| {
        declaration.implementation_owner.as_ref() == Some(owner)
            && declaration.actor_type.symbol == owner.actor_symbol
    });
    let declaration = matches
        .next()
        .ok_or(ActorInstanceStoreError::DeclarationMissing)?;
    if matches.next().is_some() {
        return Err(ActorInstanceStoreError::DeclarationAmbiguous);
    }
    Ok(declaration)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActorInstanceStoreError {
    #[error("invalid Actor logical key: {message}")]
    InvalidLogicalKey { message: String },
    #[error("Actor epoch must be positive")]
    InvalidEpoch,
    #[error("stale Actor epoch {requested}; latest materialized epoch is {latest}")]
    StaleEpoch { requested: u64, latest: u64 },
    #[error("Actor instance fence does not match the materialized instance")]
    FenceMismatch,
    #[error("Actor declaration owner does not match")]
    DeclarationOwnerMismatch,
    #[error("Actor ABI identity does not match")]
    ActorAbiMismatch,
    #[error("Actor implementation identity does not match")]
    ActorImplementationMismatch,
    #[error("Actor declaration file is missing")]
    DeclarationFileMissing,
    #[error("Actor declaration is missing")]
    DeclarationMissing,
    #[error("Actor declaration owner is ambiguous")]
    DeclarationAmbiguous,
    #[error("unsupported Actor runtime ABI {actual}")]
    UnsupportedActorRuntimeAbi { actual: String },
    #[error("unsupported Actor bootstrap encoding {actual}")]
    UnsupportedBootstrapEncoding { actual: String },
    #[error("Actor bootstrap payload decode failed: {message}")]
    BootstrapDecode { message: String },
    #[error("Actor bootstrap payload must be a record")]
    BootstrapNotRecord,
    #[error("Actor bootstrap field shape/order mismatch: expected {expected:?}, got {actual:?}")]
    BootstrapFieldShape {
        expected: Vec<String>,
        actual: Vec<String>,
    },
    #[error("Actor field {field} uses an unsupported encoding")]
    UnsupportedFieldEncoding { field: String },
    #[error("Actor field {field} type plan failed: {message}")]
    DeclarationType { field: String, message: String },
    #[error("Actor bootstrap field {field} decode failed: {message}")]
    BootstrapFieldDecode { field: String, message: String },
    #[error("Actor instance is not materialized")]
    InstanceNotFound,
    #[error("Actor instance handle was replaced")]
    InstanceReplaced,
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use serde_json::json;
    use skiff_runtime_linked_program::{
        ExternalRefTable, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedActorField,
        LinkedFileUnit, PackageUnit, RuntimeTypeContext, ServiceSymbolRef, SourceMapDto,
    };

    use super::*;

    struct ProgramFixture {
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<PackageUnit>>,
        package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
        overlay: LinkOverlay,
        types: RuntimeTypeContext,
    }

    impl ProgramFixture {
        fn view(&self) -> ProgramTypeView<'_> {
            ProgramTypeView::new(
                &self.service_files,
                &self.packages,
                &self.package_files,
                &self.overlay,
                &self.types,
            )
        }
    }

    fn owner() -> LinkedActorDeclarationOwner {
        LinkedActorDeclarationOwner {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity("file:actors".to_string()),
            actor_symbol: "DocHub".to_string(),
        }
    }

    fn abi() -> ActorAbiIdentity {
        ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:doc-hub")
    }

    fn implementation() -> ActorImplementationIdentity {
        ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:doc-hub")
    }

    fn fixture() -> ProgramFixture {
        let declaration_owner = owner();
        ProgramFixture {
            service_files: vec![Arc::new(LinkedFileUnit {
                schema_version: "skiff-file-ir-v3".to_string(),
                file_ir_identity: "file:actors".to_string(),
                source_ast_hash: "source:actors".to_string(),
                module_path: "actors".to_string(),
                ir_format_version: None,
                opcode_table_version: None,
                source_map: SourceMapDto::default(),
                declarations: FileDeclarations::default(),
                link_targets: FileLinkTargets::default(),
                actor_declarations: vec![LinkedActorDeclaration {
                    actor_type: ServiceSymbolRef {
                        module_path: "actors".to_string(),
                        symbol: "DocHub".to_string(),
                    },
                    implementation_owner: Some(declaration_owner),
                    actor_abi_identity: abi(),
                    actor_implementation_identity: implementation(),
                    actor_name: "DocHub".to_string(),
                    actor_id_type: builtin("string"),
                    fields: vec![
                        LinkedActorField {
                            name: "count".to_string(),
                            ty: builtin("integer"),
                            encoding: ActorFieldEncodingIr::CanonicalValueV1,
                        },
                        LinkedActorField {
                            name: "title".to_string(),
                            ty: builtin("string"),
                            encoding: ActorFieldEncodingIr::CanonicalValueV1,
                        },
                    ],
                    public_methods: Vec::new(),
                    actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
                }],
                types: Vec::new(),
                constants: Vec::new(),
                executables: Vec::new(),
                external_refs: ExternalRefTable::default(),
            })],
            packages: Vec::new(),
            package_files: Vec::new(),
            overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn builtin(name: &str) -> skiff_runtime_linked_program::LinkedTypeRef {
        skiff_runtime_linked_program::LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn logical_key() -> ActorLogicalKey {
        let canonical_actor_id_key_bytes = br#""doc-1""#.to_vec();
        ActorLogicalKey {
            service_id: "skiff.run/docs".to_string(),
            actor_type_identity: "service-symbol:actors.DocHub".to_string(),
            actor_id_type_identity: "builtin:string".to_string(),
            actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
            actor_id_hash: format!(
                "sha256:{}",
                hex::encode(Sha256::digest(&canonical_actor_id_key_bytes))
            ),
            canonical_actor_id_key_bytes,
        }
    }

    fn fence(epoch: u64) -> ActorInstanceFence {
        ActorInstanceFence {
            incarnation: ActorIncarnationKey {
                logical_key: logical_key(),
                epoch,
            },
            actor_abi_identity: abi(),
            actor_implementation_identity: implementation(),
            declaration_owner: owner(),
        }
    }

    fn payload() -> Vec<u8> {
        br#"{"count":7,"title":"first"}"#.to_vec()
    }

    fn request<'a>(
        program: ProgramTypeView<'a>,
        fence: ActorInstanceFence,
        payload: &'a [u8],
    ) -> ActorActivationRequest<'a> {
        ActorActivationRequest {
            fence,
            bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
            bootstrap_payload: payload,
            program,
        }
    }

    #[test]
    fn real_linked_declaration_materializes_field_frame_in_declaration_order() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .expect("valid bootstrap materializes");

        let fields = store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
                fields.to_vec()
            })
            .unwrap();
        assert_eq!(
            fields,
            vec![
                ActorFieldValue {
                    name: "count".to_string(),
                    value: RuntimeValue::Number(7.0),
                },
                ActorFieldValue {
                    name: "title".to_string(),
                    value: RuntimeValue::String("first".to_string()),
                },
            ]
        );
    }

    #[test]
    fn concurrent_activation_publishes_exactly_one_instance() {
        let fixture = fixture();
        let bytes = payload();
        let store = Arc::new(ActorInstanceStore::new());
        let barrier = Arc::new(Barrier::new(12));
        let pointers = thread::scope(|scope| {
            let joins = (0..12)
                .map(|_| {
                    let store = Arc::clone(&store);
                    let barrier = Arc::clone(&barrier);
                    let program = fixture.view();
                    let bytes = bytes.as_slice();
                    scope.spawn(move || {
                        barrier.wait();
                        let handle = store
                            .activate(request(program, fence(1), bytes))
                            .expect("concurrent activation succeeds");
                        Arc::as_ptr(&handle.instance) as usize
                    })
                })
                .collect::<Vec<_>>();
            joins
                .into_iter()
                .map(|join| join.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(pointers.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn malformed_field_shapes_and_types_fail_without_caching() {
        let fixture = fixture();
        let store = ActorInstanceStore::new();
        for malformed in [
            br#"{"title":"missing"}"#.as_slice(),
            br#"{"count":7,"extra":true,"title":"many"}"#.as_slice(),
            br#"{"title":"wrong-order","count":7}"#.as_slice(),
            br#"{"count":"wrong-type","title":"bad"}"#.as_slice(),
        ] {
            assert!(store
                .activate(request(fixture.view(), fence(1), malformed))
                .is_err());
            assert!(store.is_empty());
        }

        let bytes = payload();
        assert!(store
            .activate(request(fixture.view(), fence(1), &bytes))
            .is_ok());
    }

    #[test]
    fn declaration_and_identity_fences_fail_closed() {
        let fixture = fixture();
        let bytes = payload();
        let cases = [
            {
                let mut value = fence(1);
                value.actor_abi_identity = ActorAbiIdentity::new("wrong");
                value
            },
            {
                let mut value = fence(1);
                value.actor_implementation_identity = ActorImplementationIdentity::new("wrong");
                value
            },
            {
                let mut value = fence(1);
                value.declaration_owner.actor_symbol = "Other".to_string();
                value
            },
            fence(0),
        ];
        for bad_fence in cases {
            let store = ActorInstanceStore::new();
            assert!(store
                .activate(request(fixture.view(), bad_fence, &bytes))
                .is_err());
            assert!(store.is_empty());
        }

        let store = ActorInstanceStore::new();
        store
            .activate(request(fixture.view(), fence(2), &bytes))
            .unwrap();
        assert_eq!(
            store
                .activate(request(fixture.view(), fence(1), &bytes))
                .unwrap_err(),
            ActorInstanceStoreError::StaleEpoch {
                requested: 1,
                latest: 2
            }
        );
    }

    #[test]
    fn existing_incarnation_rejects_different_owner_abi_or_implementation() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();

        for bad_fence in [
            {
                let mut value = fence(1);
                value.actor_abi_identity = ActorAbiIdentity::new("different");
                value
            },
            {
                let mut value = fence(1);
                value.actor_implementation_identity = ActorImplementationIdentity::new("different");
                value
            },
            {
                let mut value = fence(1);
                value.declaration_owner.file = FileAddr::FileIrIdentity("file:other".to_string());
                value
            },
        ] {
            assert_eq!(
                store
                    .activate(request(fixture.view(), bad_fence, &bytes))
                    .unwrap_err(),
                ActorInstanceStoreError::FenceMismatch
            );
        }
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn unknown_encoding_and_invalid_json_leave_no_partial_instance() {
        let fixture = fixture();
        let store = ActorInstanceStore::new();
        assert_eq!(
            store
                .activate(ActorActivationRequest {
                    fence: fence(1),
                    bootstrap_encoding_version: "unknown",
                    bootstrap_payload: &payload(),
                    program: fixture.view(),
                })
                .unwrap_err(),
            ActorInstanceStoreError::UnsupportedBootstrapEncoding {
                actual: "unknown".to_string()
            }
        );
        assert!(store
            .activate(request(fixture.view(), fence(1), b"{"))
            .is_err());
        assert!(store.is_empty());
    }

    #[test]
    fn discard_requires_exact_fence_and_old_fence_cannot_remove_new_incarnation() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let old = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let new = store
            .activate(request(fixture.view(), fence(2), &bytes))
            .unwrap();
        assert_eq!(
            store
                .with_fields_for_executor(&ActorExecutorAuthority::new(), &old, |fields, _| fields
                    .len())
                .unwrap_err(),
            ActorInstanceStoreError::StaleEpoch {
                requested: 1,
                latest: 2
            }
        );

        let mut forged = new.clone();
        forged.fence.actor_implementation_identity = ActorImplementationIdentity::new("different");
        assert!(!store.discard_exact(&forged));
        assert!(store.discard_exact(&old));
        assert_eq!(store.len(), 1);
        assert!(!store.discard_exact(&old));
        assert!(store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &new, |fields, _| fields
                .len())
            .is_ok());
    }

    #[test]
    fn stale_cleanup_handle_cannot_remove_same_epoch_rematerialization() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let old = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        assert!(store.discard_exact(&old));
        let current = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        assert!(!store.discard_exact(&old));
        assert!(store.discard_exact(&current));
    }

    #[test]
    fn live_field_mutation_never_changes_registry_bootstrap_bytes() {
        let fixture = fixture();
        let registry_payload = payload();
        let original = registry_payload.clone();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &registry_payload))
            .unwrap();
        store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
                fields[0].value = RuntimeValue::Number(99.0);
            })
            .unwrap();
        assert_eq!(registry_payload, original);
        let count = store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
                fields[0].value.clone()
            })
            .unwrap();
        assert_eq!(count, RuntimeValue::Number(99.0));
    }

    #[test]
    fn logical_key_is_part_of_the_incarnation_identity() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let mut other = fence(1);
        other.incarnation.logical_key.canonical_actor_id_key_bytes =
            serde_json::to_vec(&json!("doc-2")).unwrap();
        other.incarnation.logical_key.actor_id_hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                &other.incarnation.logical_key.canonical_actor_id_key_bytes
            ))
        );
        store
            .activate(request(fixture.view(), other, &bytes))
            .unwrap();
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn empty_logical_key_components_are_rejected() {
        let fixture = fixture();
        let bytes = payload();
        let mut invalid = fence(1);
        invalid.incarnation.logical_key.service_id.clear();
        assert!(matches!(
            ActorInstanceStore::new().activate(request(fixture.view(), invalid, &bytes)),
            Err(ActorInstanceStoreError::InvalidLogicalKey { .. })
        ));
    }

    #[test]
    fn bootstrap_object_order_is_canonical_not_declaration_storage_order() {
        let fixture = fixture();
        let store = ActorInstanceStore::new();
        let mut object = serde_json::Map::new();
        object.insert("title".to_string(), json!("late"));
        object.insert("count".to_string(), json!(1));
        let non_canonical = serde_json::to_vec(&Value::Object(object)).unwrap();
        assert!(matches!(
            store.activate(request(fixture.view(), fence(1), &non_canonical)),
            Err(ActorInstanceStoreError::BootstrapFieldShape { .. })
        ));
    }

    #[test]
    fn declaration_without_exact_owner_is_rejected() {
        let mut fixture = fixture();
        fixture.service_files[0] = Arc::new({
            let mut file = fixture.service_files[0].as_ref().clone();
            file.actor_declarations[0].implementation_owner = None;
            file
        });
        let bytes = payload();
        assert_eq!(
            ActorInstanceStore::new()
                .activate(request(fixture.view(), fence(1), &bytes))
                .unwrap_err(),
            ActorInstanceStoreError::DeclarationMissing
        );
    }
}
