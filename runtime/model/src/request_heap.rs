use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::{
    error::{Result, RuntimeModelError as RuntimeError},
    service_error::{CatchIdentity, RequestExceptionCause},
    value::{
        ActorRef, HeapHandle, HeapNode, InterfaceCarrier, InterfaceMethodTable,
        InterfaceMethodTarget, InterfaceValue, RuntimeBytes, RuntimeMap, RuntimeObject,
        RuntimeObjectFields, RuntimeValue, RuntimeValueCarrier, RuntimeValueKey,
    },
};

const INITIAL_GENERATION: u32 = 0;
const NODE_OVERHEAD_BYTES: usize = 32;
const ARRAY_ITEM_OVERHEAD_BYTES: usize = 8;
const OBJECT_FIELD_OVERHEAD_BYTES: usize = 16;
const MAP_ENTRY_OVERHEAD_BYTES: usize = 24;
const INTERFACE_VALUE_OVERHEAD_BYTES: usize = 48;
const INTERFACE_METHOD_SLOT_OVERHEAD_BYTES: usize = 32;
const HANDLE_BYTES: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestHeapLimits {
    pub max_nodes: usize,
    pub max_estimated_bytes: usize,
    pub max_clone_depth: usize,
    pub max_materialize_depth: usize,
    pub max_materialize_output_bytes: usize,
}

impl Default for RequestHeapLimits {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
            max_estimated_bytes: 512 * 1024 * 1024,
            max_clone_depth: 512,
            max_materialize_depth: 512,
            max_materialize_output_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestHeapStats {
    pub node_count: usize,
    pub estimated_bytes: usize,
    pub clone_depth: usize,
    pub materialize_depth: usize,
    pub materialize_output_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestHeapCheckpoint {
    len: usize,
    stats: RequestHeapStats,
}

#[derive(Clone, Debug)]
pub struct PreparedRequestHeapRollback {
    nodes: Vec<HeapSlot>,
    stats: RequestHeapStats,
    rebased_roots: Vec<RuntimeValue>,
}

#[derive(Debug)]
pub enum RequestHeapRollbackRebaseError {
    ResourceLimit(RuntimeError),
    InvalidSource(RuntimeError),
}

impl RequestHeapRollbackRebaseError {
    pub fn is_skippable(&self) -> bool {
        matches!(self, Self::ResourceLimit(_))
    }

    pub fn into_runtime_error(self) -> RuntimeError {
        match self {
            Self::ResourceLimit(error) | Self::InvalidSource(error) => error,
        }
    }

    fn classify(error: RuntimeError) -> Self {
        match error {
            error @ RuntimeError::ResourceLimitExceeded { .. } => Self::ResourceLimit(error),
            error => Self::InvalidSource(error),
        }
    }
}

impl std::fmt::Display for RequestHeapRollbackRebaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResourceLimit(error) | Self::InvalidSource(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RequestHeapRollbackRebaseError {}

impl PreparedRequestHeapRollback {
    pub fn rebased_roots(&self) -> &[RuntimeValue] {
        &self.rebased_roots
    }
}

#[derive(Clone, Debug)]
pub struct RequestHeap {
    nodes: Vec<HeapSlot>,
    limits: RequestHeapLimits,
    stats: RequestHeapStats,
    epoch: u32,
}

impl RequestHeap {
    pub fn new(limits: RequestHeapLimits) -> Self {
        Self::new_with_epoch(0, limits)
    }

    /// Creates a heap whose allocated handles are stamped with `epoch`.
    ///
    /// Actor instance compaction replaces the whole arena with a fresh heap at
    /// epoch + 1, so stale handles from the previous arena fail closed in
    /// `slot`/`slot_mut`. Ordinary requests use epoch 0.
    pub fn new_with_epoch(epoch: u32, limits: RequestHeapLimits) -> Self {
        Self {
            nodes: Vec::new(),
            limits,
            stats: RequestHeapStats::default(),
            epoch,
        }
    }

    pub fn limits(&self) -> &RequestHeapLimits {
        &self.limits
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn stats(&self) -> RequestHeapStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn checkpoint(&self) -> RequestHeapCheckpoint {
        RequestHeapCheckpoint {
            len: self.nodes.len(),
            stats: self.stats,
        }
    }

    pub fn rollback_to_checkpoint(&mut self, checkpoint: RequestHeapCheckpoint) {
        self.nodes.truncate(checkpoint.len);
        self.stats = checkpoint.stats;
    }

    /// Prepares a rollback that preserves every pre-checkpoint heap handle while
    /// retaining post-checkpoint nodes still reachable from the pinned prefix or
    /// the supplied roots.
    ///
    /// Preparation is transactional: the source heap is only read. The returned
    /// plan can be installed with [`RequestHeap::commit_prepared_rollback_rebase`].
    pub fn prepare_rollback_rebase(
        &self,
        checkpoint: RequestHeapCheckpoint,
        explicit_roots: &[RuntimeValue],
    ) -> std::result::Result<PreparedRequestHeapRollback, RequestHeapRollbackRebaseError> {
        if checkpoint.len > self.nodes.len() {
            return Err(RequestHeapRollbackRebaseError::InvalidSource(
                RuntimeError::Decode(format!(
                    "request heap checkpoint length {} exceeds current heap length {}",
                    checkpoint.len,
                    self.nodes.len()
                )),
            ));
        }

        let mut builder = RollbackRebaseBuilder::new(self, checkpoint);
        builder
            .discover(explicit_roots)
            .and_then(|()| builder.prepare(explicit_roots))
            .map_err(RequestHeapRollbackRebaseError::classify)
    }

    pub fn commit_prepared_rollback_rebase(&mut self, prepared: PreparedRequestHeapRollback) {
        self.nodes = prepared.nodes;
        self.stats = prepared.stats;
    }

    pub fn alloc_array(&mut self, items: Vec<RuntimeValue>) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Array(items))
    }

    pub fn alloc_array_carriers(&mut self, items: Vec<RuntimeValueCarrier>) -> Result<HeapHandle> {
        let (values, identities) = split_carriers(items);
        self.alloc_node_with_carriers(
            HeapNode::Array(values),
            HeapCarrierLayout::Array(identities),
        )
    }

    /// Allocates an internal one-value cell that retains the carrier identity.
    ///
    /// The cell is a request-local handoff primitive for owners such as the
    /// in-process stream runtime. It is not an ordinary language Array and can
    /// only be projected through [`RequestHeap::local_carrier_cell`].
    pub fn alloc_local_carrier_cell(&mut self, carrier: RuntimeValueCarrier) -> Result<HeapHandle> {
        let (value, identity) = carrier.into_parts();
        self.alloc_node_with_carriers(
            HeapNode::Array(vec![value]),
            HeapCarrierLayout::LocalCarrierCell(identity),
        )
    }

    pub fn alloc_bytes(&mut self, bytes: impl Into<RuntimeBytes>) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Bytes(bytes.into()))
    }

    pub fn alloc_object(&mut self, object: RuntimeObject) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Object(object))
    }

    pub fn alloc_object_carriers(
        &mut self,
        fields: BTreeMap<String, RuntimeValueCarrier>,
    ) -> Result<HeapHandle> {
        let mut values = RuntimeObjectFields::new();
        let mut identities = BTreeMap::new();
        for (field, carrier) in fields {
            let (value, identity) = carrier.into_parts();
            values.insert(field.clone(), value);
            identities.insert(field, identity);
        }
        self.alloc_node_with_carriers(
            HeapNode::Object(RuntimeObject::unshaped(values)),
            HeapCarrierLayout::Object(identities),
        )
    }

    pub fn alloc_map(&mut self, map: RuntimeMap) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Map(map))
    }

    pub fn alloc_map_carriers(
        &mut self,
        entries: BTreeMap<RuntimeValueKey, RuntimeValueCarrier>,
    ) -> Result<HeapHandle> {
        let mut values = RuntimeMap::new();
        let mut identities = BTreeMap::new();
        for (key, carrier) in entries {
            let (value, identity) = carrier.into_parts();
            values.insert(key.clone(), value);
            identities.insert(key, identity);
        }
        self.alloc_node_with_carriers(HeapNode::Map(values), HeapCarrierLayout::Map(identities))
    }

    pub fn alloc_interface(&mut self, value: InterfaceValue) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Interface(value))
    }

    pub fn alloc_interface_with_local_payload_identity(
        &mut self,
        value: InterfaceValue,
        payload_identity: Option<CatchIdentity>,
    ) -> Result<HeapHandle> {
        self.alloc_node_with_carriers(
            HeapNode::Interface(value),
            HeapCarrierLayout::Interface(payload_identity),
        )
    }

    pub fn alloc_exception(
        &mut self,
        exception: crate::service_error::RequestException,
    ) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Exception(exception))
    }

    pub fn get(&self, handle: HeapHandle) -> Result<&HeapNode> {
        self.slot(handle).map(|slot| &slot.node)
    }

    pub fn array_item_carrier(
        &self,
        handle: HeapHandle,
        index: usize,
    ) -> Result<Option<RuntimeValueCarrier>> {
        let slot = self.slot(handle)?;
        let HeapNode::Array(items) = &slot.node else {
            return Ok(None);
        };
        if matches!(&slot.carriers, HeapCarrierLayout::LocalCarrierCell(_)) {
            return Ok(None);
        }
        let Some(value) = items.get(index) else {
            return Ok(None);
        };
        let identity = match &slot.carriers {
            HeapCarrierLayout::Array(identities) => identities.get(index).cloned().flatten(),
            _ => None,
        };
        Ok(Some(RuntimeValueCarrier::from_parts(
            value.clone(),
            identity,
        )))
    }

    pub fn local_carrier_cell(&self, handle: HeapHandle) -> Result<Option<RuntimeValueCarrier>> {
        let slot = self.slot(handle)?;
        let (HeapNode::Array(items), HeapCarrierLayout::LocalCarrierCell(identity)) =
            (&slot.node, &slot.carriers)
        else {
            return Ok(None);
        };
        let Some(value) = items.first() else {
            return Err(RuntimeError::Decode(
                "request-local carrier cell is empty".to_string(),
            ));
        };
        Ok(Some(RuntimeValueCarrier::from_parts(
            value.clone(),
            identity.clone(),
        )))
    }

    pub fn object_field_carrier(
        &self,
        handle: HeapHandle,
        field: &str,
    ) -> Result<Option<RuntimeValueCarrier>> {
        let slot = self.slot(handle)?;
        let (value, identity) = match &slot.node {
            HeapNode::Object(object) => {
                let Some(value) = object.fields().get(field) else {
                    return Ok(None);
                };
                let identity = match &slot.carriers {
                    HeapCarrierLayout::Object(identities) => {
                        identities.get(field).cloned().flatten()
                    }
                    _ => None,
                };
                (value, identity)
            }
            HeapNode::Map(map) => {
                let key = RuntimeValueKey::string(field);
                let Some(value) = map.get(&key) else {
                    return Ok(None);
                };
                let identity = match &slot.carriers {
                    HeapCarrierLayout::Map(identities) => identities.get(&key).cloned().flatten(),
                    _ => None,
                };
                (value, identity)
            }
            _ => return Ok(None),
        };
        Ok(Some(RuntimeValueCarrier::from_parts(
            value.clone(),
            identity,
        )))
    }

    pub fn map_entry_carrier(
        &self,
        handle: HeapHandle,
        key: &RuntimeValueKey,
    ) -> Result<Option<RuntimeValueCarrier>> {
        let slot = self.slot(handle)?;
        match &slot.node {
            HeapNode::Map(map) => {
                let Some(value) = map.get(key) else {
                    return Ok(None);
                };
                let identity = match &slot.carriers {
                    HeapCarrierLayout::Map(identities) => identities.get(key).cloned().flatten(),
                    _ => None,
                };
                Ok(Some(RuntimeValueCarrier::from_parts(
                    value.clone(),
                    identity,
                )))
            }
            HeapNode::Object(_) => self.object_field_carrier(handle, key.string_payload()),
            _ => Ok(None),
        }
    }

    pub fn interface_local_payload_carrier(
        &self,
        handle: HeapHandle,
    ) -> Result<Option<RuntimeValueCarrier>> {
        let slot = self.slot(handle)?;
        let HeapNode::Interface(interface) = &slot.node else {
            return Ok(None);
        };
        let InterfaceCarrier::Local { payload, .. } = interface.carrier() else {
            return Ok(None);
        };
        let identity = match &slot.carriers {
            HeapCarrierLayout::Interface(identity) => identity.clone(),
            _ => None,
        };
        Ok(Some(RuntimeValueCarrier::from_parts(
            payload.clone(),
            identity,
        )))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn push_array_item_without_cycle_check_for_test(
        &mut self,
        handle: HeapHandle,
        value: RuntimeValue,
    ) -> Result<()> {
        self.push_array_item_inner(handle, value)
    }

    pub fn push_array_item(&mut self, handle: HeapHandle, value: RuntimeValue) -> Result<()> {
        self.push_array_item_carrier(handle, value.into())
    }

    pub fn push_array_item_carrier(
        &mut self,
        handle: HeapHandle,
        value: RuntimeValueCarrier,
    ) -> Result<()> {
        self.check_insert_without_cycle(handle, value.value())?;
        self.push_array_item_carrier_inner(handle, value)
    }

    pub fn set_array_item(
        &mut self,
        handle: HeapHandle,
        index: usize,
        value: RuntimeValue,
    ) -> Result<()> {
        self.set_array_item_carrier(handle, index, value.into())
    }

    pub fn set_array_item_carrier(
        &mut self,
        handle: HeapHandle,
        index: usize,
        value: RuntimeValueCarrier,
    ) -> Result<()> {
        self.check_insert_without_cycle(handle, value.value())?;
        let (value, identity) = value.into_parts();
        let (old_bytes, new_bytes) = {
            let slot = self.slot(handle)?;
            let HeapNode::Array(items) = &slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.set target is not an array".to_string(),
                ));
            };
            let Some(old_value) = items.get(index) else {
                return Err(RuntimeError::Decode(format!(
                    "Array.set index {index} is out of bounds"
                )));
            };
            (
                estimate_array_item_bytes(old_value),
                estimate_array_item_bytes(&value),
            )
        };
        self.check_estimated_bytes_replacement(old_bytes, new_bytes)?;

        {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Array(items) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.set target is not an array".to_string(),
                ));
            };
            items[index] = value;
            match &mut slot.carriers {
                HeapCarrierLayout::Array(identities) => {
                    identities[index] = identity;
                }
                carriers => {
                    *carriers = HeapCarrierLayout::Array(array_identity_slots(items.len()));
                    let HeapCarrierLayout::Array(identities) = carriers else {
                        unreachable!("array carrier layout was just initialized");
                    };
                    identities[index] = identity;
                }
            }
            apply_estimated_bytes_replacement(&mut slot.estimated_bytes, old_bytes, new_bytes);
        }
        self.apply_stats_estimated_bytes_replacement(old_bytes, new_bytes);
        Ok(())
    }

    pub fn pop_array_item(&mut self, handle: HeapHandle) -> Result<RuntimeValue> {
        self.pop_array_item_carrier(handle)
            .map(RuntimeValueCarrier::into_value)
    }

    pub fn pop_array_item_carrier(&mut self, handle: HeapHandle) -> Result<RuntimeValueCarrier> {
        let popped_bytes = {
            let slot = self.slot(handle)?;
            let HeapNode::Array(items) = &slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.pop target is not an array".to_string(),
                ));
            };
            items.last().map(estimate_array_item_bytes).unwrap_or(0)
        };

        let (popped, identity) = {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Array(items) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.pop target is not an array".to_string(),
                ));
            };
            let popped = items.pop().unwrap_or(RuntimeValue::Null);
            let identity = match &mut slot.carriers {
                HeapCarrierLayout::Array(identities) => identities.pop().flatten(),
                _ => None,
            };
            if popped_bytes > 0 {
                slot.estimated_bytes = slot.estimated_bytes.saturating_sub(popped_bytes);
            }
            (popped, identity)
        };
        if popped_bytes > 0 {
            self.stats.estimated_bytes = self.stats.estimated_bytes.saturating_sub(popped_bytes);
        }
        Ok(RuntimeValueCarrier::from_parts(popped, identity))
    }

    pub fn set_map_entry(
        &mut self,
        handle: HeapHandle,
        key: RuntimeValueKey,
        value: RuntimeValue,
    ) -> Result<bool> {
        self.set_map_entry_carrier(handle, key, value.into())
    }

    pub fn set_map_entry_carrier(
        &mut self,
        handle: HeapHandle,
        key: RuntimeValueKey,
        value: RuntimeValueCarrier,
    ) -> Result<bool> {
        self.check_insert_without_cycle(handle, value.value())?;
        let (value, identity) = value.into_parts();
        let plan = {
            let slot = self.slot(handle)?;
            match &slot.node {
                HeapNode::Map(map) => {
                    let old_value = map.get(&key);
                    let existed = old_value.is_some();
                    let old_bytes = old_value
                        .map(|old_value| estimate_map_entry_bytes(&key, old_value))
                        .unwrap_or(0);
                    let new_bytes = estimate_map_entry_bytes(&key, &value);
                    MapEntrySetPlan::Map {
                        existed,
                        old_bytes,
                        new_bytes,
                    }
                }
                HeapNode::Object(object) => {
                    let field = object_field_key_from_map_key(&key)?.to_string();
                    let old_value = object.fields().get(&field);
                    let existed = old_value.is_some();
                    let old_bytes = old_value
                        .map(|old_value| estimate_object_field_bytes(&field, old_value))
                        .unwrap_or(0);
                    let new_bytes = estimate_object_field_bytes(&field, &value);
                    MapEntrySetPlan::ObjectField {
                        field,
                        existed,
                        old_bytes,
                        new_bytes,
                    }
                }
                _ => {
                    return Err(RuntimeError::Decode(
                        "Map target is not an object".to_string(),
                    ));
                }
            }
        };
        let identity_key = match &plan {
            MapEntrySetPlan::Map { .. } => CollectionIdentityKey::Map(key.clone()),
            MapEntrySetPlan::ObjectField { field, .. } => {
                CollectionIdentityKey::Object(field.clone())
            }
        };
        let existed = match plan {
            MapEntrySetPlan::Map {
                existed,
                old_bytes,
                new_bytes,
            } => self.set_map_entry_with_bytes(handle, key, value, existed, old_bytes, new_bytes),
            MapEntrySetPlan::ObjectField {
                field,
                existed,
                old_bytes,
                new_bytes,
            } => self
                .set_object_field_with_bytes(handle, field, value, existed, old_bytes, new_bytes),
        }?;
        self.set_collection_identity(handle, identity_key, identity)?;
        Ok(existed)
    }

    pub fn delete_map_entry(&mut self, handle: HeapHandle, key: &RuntimeValueKey) -> Result<bool> {
        let plan = {
            let slot = self.slot(handle)?;
            match &slot.node {
                HeapNode::Map(map) => MapEntryDeletePlan::Map {
                    old_bytes: map
                        .get(key)
                        .map(|old_value| estimate_map_entry_bytes(key, old_value))
                        .unwrap_or(0),
                },
                HeapNode::Object(_) => MapEntryDeletePlan::ObjectField {
                    field: object_field_key_from_map_key(key)?.to_string(),
                },
                _ => {
                    return Err(RuntimeError::Decode(
                        "Map target is not an object".to_string(),
                    ));
                }
            }
        };
        let old_bytes = match plan {
            MapEntryDeletePlan::Map { old_bytes } => old_bytes,
            MapEntryDeletePlan::ObjectField { field } => {
                return self.delete_object_field(handle, &field);
            }
        };

        let existed = {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Map(map) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Map target is not an object".to_string(),
                ));
            };
            let existed = map.remove(key).is_some();
            if let HeapCarrierLayout::Map(identities) = &mut slot.carriers {
                identities.remove(key);
            }
            if old_bytes > 0 {
                slot.estimated_bytes = slot.estimated_bytes.saturating_sub(old_bytes);
            }
            existed
        };
        if old_bytes > 0 {
            self.stats.estimated_bytes = self.stats.estimated_bytes.saturating_sub(old_bytes);
        }
        Ok(existed)
    }

    pub fn set_object_field(
        &mut self,
        handle: HeapHandle,
        field: String,
        value: RuntimeValue,
    ) -> Result<bool> {
        self.set_object_field_carrier(handle, field, value.into())
    }

    pub fn set_object_field_carrier(
        &mut self,
        handle: HeapHandle,
        field: String,
        value: RuntimeValueCarrier,
    ) -> Result<bool> {
        self.check_insert_without_cycle(handle, value.value())?;
        let (value, identity) = value.into_parts();
        let plan = {
            let slot = self.slot(handle)?;
            match &slot.node {
                HeapNode::Object(object) => {
                    let old_value = object.fields().get(&field);
                    let existed = old_value.is_some();
                    let old_bytes = old_value
                        .map(|old_value| estimate_object_field_bytes(&field, old_value))
                        .unwrap_or(0);
                    let new_bytes = estimate_object_field_bytes(&field, &value);
                    ObjectFieldSetPlan::Object {
                        existed,
                        old_bytes,
                        new_bytes,
                    }
                }
                HeapNode::Map(map) => {
                    let key = RuntimeValueKey::string(&field);
                    let old_value = map.get(&key);
                    let existed = old_value.is_some();
                    let old_bytes = old_value
                        .map(|old_value| estimate_map_entry_bytes(&key, old_value))
                        .unwrap_or(0);
                    let new_bytes = estimate_map_entry_bytes(&key, &value);
                    ObjectFieldSetPlan::MapEntry {
                        key,
                        existed,
                        old_bytes,
                        new_bytes,
                    }
                }
                _ => {
                    return Err(RuntimeError::Decode(
                        "field assignment target is not an object".to_string(),
                    ));
                }
            }
        };
        let identity_key = match &plan {
            ObjectFieldSetPlan::Object { .. } => CollectionIdentityKey::Object(field.clone()),
            ObjectFieldSetPlan::MapEntry { key, .. } => CollectionIdentityKey::Map(key.clone()),
        };
        let existed = match plan {
            ObjectFieldSetPlan::Object {
                existed,
                old_bytes,
                new_bytes,
            } => self
                .set_object_field_with_bytes(handle, field, value, existed, old_bytes, new_bytes),
            ObjectFieldSetPlan::MapEntry {
                key,
                existed,
                old_bytes,
                new_bytes,
            } => self.set_map_entry_with_bytes(handle, key, value, existed, old_bytes, new_bytes),
        }?;
        self.set_collection_identity(handle, identity_key, identity)?;
        Ok(existed)
    }

    fn set_map_entry_with_bytes(
        &mut self,
        handle: HeapHandle,
        key: RuntimeValueKey,
        value: RuntimeValue,
        existed: bool,
        old_bytes: usize,
        new_bytes: usize,
    ) -> Result<bool> {
        self.check_estimated_bytes_replacement(old_bytes, new_bytes)?;

        {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Map(map) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Map target is not an object".to_string(),
                ));
            };
            debug_assert_eq!(map.insert(key, value).is_some(), existed);
            apply_estimated_bytes_replacement(&mut slot.estimated_bytes, old_bytes, new_bytes);
        }
        self.apply_stats_estimated_bytes_replacement(old_bytes, new_bytes);
        Ok(existed)
    }

    fn set_object_field_with_bytes(
        &mut self,
        handle: HeapHandle,
        field: String,
        value: RuntimeValue,
        existed: bool,
        old_bytes: usize,
        new_bytes: usize,
    ) -> Result<bool> {
        self.check_estimated_bytes_replacement(old_bytes, new_bytes)?;

        {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Object(object) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "field assignment target is not an object".to_string(),
                ));
            };
            debug_assert_eq!(object.fields_mut().insert(field, value).is_some(), existed);
            apply_estimated_bytes_replacement(&mut slot.estimated_bytes, old_bytes, new_bytes);
        }
        self.apply_stats_estimated_bytes_replacement(old_bytes, new_bytes);
        Ok(existed)
    }

    pub fn delete_object_field(&mut self, handle: HeapHandle, field: &str) -> Result<bool> {
        let plan = {
            let slot = self.slot(handle)?;
            match &slot.node {
                HeapNode::Object(object) => ObjectFieldDeletePlan::Object {
                    old_bytes: object
                        .fields()
                        .get(field)
                        .map(|old_value| estimate_object_field_bytes(field, old_value))
                        .unwrap_or(0),
                },
                HeapNode::Map(map) => {
                    let key = RuntimeValueKey::string(field);
                    ObjectFieldDeletePlan::MapEntry {
                        old_bytes: map
                            .get(&key)
                            .map(|old_value| estimate_map_entry_bytes(&key, old_value))
                            .unwrap_or(0),
                        key,
                    }
                }
                _ => {
                    return Err(RuntimeError::Decode(
                        "Map target is not an object".to_string(),
                    ));
                }
            }
        };
        let old_bytes = match plan {
            ObjectFieldDeletePlan::Object { old_bytes } => old_bytes,
            ObjectFieldDeletePlan::MapEntry { key, old_bytes } => {
                return self.delete_map_entry_with_bytes(handle, &key, old_bytes);
            }
        };

        self.delete_object_field_with_bytes(handle, field, old_bytes)
    }

    fn delete_map_entry_with_bytes(
        &mut self,
        handle: HeapHandle,
        key: &RuntimeValueKey,
        old_bytes: usize,
    ) -> Result<bool> {
        let existed = {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Map(map) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Map target is not an object".to_string(),
                ));
            };
            let existed = map.remove(key).is_some();
            if let HeapCarrierLayout::Map(identities) = &mut slot.carriers {
                identities.remove(key);
            }
            if old_bytes > 0 {
                slot.estimated_bytes = slot.estimated_bytes.saturating_sub(old_bytes);
            }
            existed
        };
        if old_bytes > 0 {
            self.stats.estimated_bytes = self.stats.estimated_bytes.saturating_sub(old_bytes);
        }
        Ok(existed)
    }

    fn delete_object_field_with_bytes(
        &mut self,
        handle: HeapHandle,
        field: &str,
        old_bytes: usize,
    ) -> Result<bool> {
        let existed = {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Object(object) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Map target is not an object".to_string(),
                ));
            };
            let existed = object.fields_mut().remove(field).is_some();
            if let HeapCarrierLayout::Object(identities) = &mut slot.carriers {
                identities.remove(field);
            }
            if old_bytes > 0 {
                slot.estimated_bytes = slot.estimated_bytes.saturating_sub(old_bytes);
            }
            existed
        };
        if old_bytes > 0 {
            self.stats.estimated_bytes = self.stats.estimated_bytes.saturating_sub(old_bytes);
        }
        Ok(existed)
    }

    pub fn recompute_stats(&mut self) -> RequestHeapStats {
        let estimated_bytes = self
            .nodes
            .iter_mut()
            .map(|slot| {
                slot.estimated_bytes = estimate_heap_node_bytes(&slot.node);
                slot.estimated_bytes
            })
            .fold(0usize, usize::saturating_add);
        self.stats.node_count = self.nodes.len();
        self.stats.estimated_bytes = estimated_bytes;
        self.stats
    }

    pub fn record_clone_depth(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_clone_depth {
            return Err(resource_limit_error(
                "max clone depth",
                self.limits.max_clone_depth,
                self.stats.clone_depth,
                depth.saturating_sub(self.stats.clone_depth),
            ));
        }
        self.stats.clone_depth = self.stats.clone_depth.max(depth);
        Ok(())
    }

    pub fn record_materialize_stats(&mut self, depth: usize, output_bytes: usize) -> Result<()> {
        if depth > self.limits.max_materialize_depth {
            return Err(resource_limit_error(
                "max materialize depth",
                self.limits.max_materialize_depth,
                self.stats.materialize_depth,
                depth.saturating_sub(self.stats.materialize_depth),
            ));
        }
        if output_bytes > self.limits.max_materialize_output_bytes {
            return Err(resource_limit_error(
                "max materialize output bytes",
                self.limits.max_materialize_output_bytes,
                self.stats.materialize_output_bytes,
                output_bytes.saturating_sub(self.stats.materialize_output_bytes),
            ));
        }
        self.stats.materialize_depth = self.stats.materialize_depth.max(depth);
        self.stats.materialize_output_bytes = self.stats.materialize_output_bytes.max(output_bytes);
        Ok(())
    }

    pub fn record_materialized_value(&mut self, depth: usize, value: &Value) -> Result<()> {
        let output_bytes = serde_json::to_vec(value)?.len();
        self.record_materialize_stats(depth, output_bytes)
    }

    pub fn can_insert_without_cycle(
        &self,
        target: HeapHandle,
        inserted: &RuntimeValue,
    ) -> Result<bool> {
        let Some(inserted_handle) = inserted.as_heap_handle() else {
            return Ok(true);
        };
        Ok(!self.is_reachable(inserted_handle, target)?)
    }

    pub fn check_insert_without_cycle(
        &self,
        target: HeapHandle,
        inserted: &RuntimeValue,
    ) -> Result<()> {
        if self.can_insert_without_cycle(target, inserted)? {
            Ok(())
        } else {
            Err(RuntimeError::Decode(format!(
                "heap mutation would create a cycle by inserting {inserted:?} into {target}"
            )))
        }
    }

    pub fn is_reachable(&self, start: HeapHandle, target: HeapHandle) -> Result<bool> {
        let mut visiting = HashSet::new();
        self.is_reachable_inner(start, target, &mut visiting)
    }

    fn alloc_node(&mut self, node: HeapNode) -> Result<HeapHandle> {
        let carriers = HeapCarrierLayout::for_node(&node);
        self.alloc_node_with_carriers(node, carriers)
    }

    fn alloc_node_with_carriers(
        &mut self,
        node: HeapNode,
        carriers: HeapCarrierLayout,
    ) -> Result<HeapHandle> {
        let estimated_bytes = estimate_heap_node_bytes(&node);
        self.check_node_limit()?;
        self.check_estimated_bytes_limit(estimated_bytes)?;
        if self.nodes.len() >= u32::MAX as usize {
            return Err(resource_limit_error(
                "heap handle index space",
                u32::MAX as usize,
                self.nodes.len(),
                1,
            ));
        }

        let handle =
            HeapHandle::new_with_epoch(self.nodes.len() as u32, INITIAL_GENERATION, self.epoch);
        self.nodes.push(HeapSlot {
            generation: INITIAL_GENERATION,
            estimated_bytes,
            node,
            carriers,
        });
        self.stats.node_count += 1;
        self.stats.estimated_bytes = self.stats.estimated_bytes.saturating_add(estimated_bytes);
        Ok(handle)
    }

    fn check_node_limit(&self) -> Result<()> {
        let requested = self.stats.node_count.saturating_add(1);
        if requested > self.limits.max_nodes {
            return Err(resource_limit_error(
                "max heap nodes",
                self.limits.max_nodes,
                self.stats.node_count,
                1,
            ));
        }
        Ok(())
    }

    fn check_estimated_bytes_limit(&self, estimated_bytes: usize) -> Result<()> {
        let requested = self.stats.estimated_bytes.saturating_add(estimated_bytes);
        if requested > self.limits.max_estimated_bytes {
            return Err(resource_limit_error(
                "max estimated heap bytes",
                self.limits.max_estimated_bytes,
                self.stats.estimated_bytes,
                estimated_bytes,
            ));
        }
        Ok(())
    }

    fn check_updated_estimated_bytes_limit(
        &self,
        old_bytes: usize,
        new_bytes: usize,
    ) -> Result<()> {
        let requested = self
            .stats
            .estimated_bytes
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
        if requested > self.limits.max_estimated_bytes {
            return Err(resource_limit_error(
                "max estimated heap bytes",
                self.limits.max_estimated_bytes,
                self.stats.estimated_bytes.saturating_sub(old_bytes),
                new_bytes,
            ));
        }
        Ok(())
    }

    fn check_estimated_bytes_replacement(&self, old_bytes: usize, new_bytes: usize) -> Result<()> {
        self.check_updated_estimated_bytes_limit(old_bytes, new_bytes)
    }

    fn apply_stats_estimated_bytes_replacement(&mut self, old_bytes: usize, new_bytes: usize) {
        self.stats.estimated_bytes = self
            .stats
            .estimated_bytes
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
    }

    #[cfg(any(test, feature = "test-support"))]
    fn push_array_item_inner(&mut self, handle: HeapHandle, value: RuntimeValue) -> Result<()> {
        self.push_array_item_carrier_inner(handle, value.into())
    }

    fn push_array_item_carrier_inner(
        &mut self,
        handle: HeapHandle,
        value: RuntimeValueCarrier,
    ) -> Result<()> {
        let (value, identity) = value.into_parts();
        let item_bytes = {
            let slot = self.slot(handle)?;
            let HeapNode::Array(_) = &slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.push target is not an array".to_string(),
                ));
            };
            estimate_array_item_bytes(&value)
        };
        self.check_estimated_bytes_replacement(0, item_bytes)?;

        {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Array(items) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.push target is not an array".to_string(),
                ));
            };
            items.push(value);
            match &mut slot.carriers {
                HeapCarrierLayout::Array(identities) => identities.push(identity),
                carriers => {
                    let mut identities = array_identity_slots(items.len().saturating_sub(1));
                    identities.push(identity);
                    *carriers = HeapCarrierLayout::Array(identities);
                }
            }
            slot.estimated_bytes = slot.estimated_bytes.saturating_add(item_bytes);
        }
        self.stats.estimated_bytes = self.stats.estimated_bytes.saturating_add(item_bytes);
        Ok(())
    }

    fn is_reachable_inner(
        &self,
        start: HeapHandle,
        target: HeapHandle,
        visiting: &mut HashSet<HeapHandle>,
    ) -> Result<bool> {
        if start == target {
            return Ok(true);
        }
        if !visiting.insert(start) {
            return Ok(false);
        }

        let reachable = match self.get(start)? {
            HeapNode::Bytes(_) => false,
            HeapNode::Array(items) => {
                self.values_contain_reachable(items.iter(), target, visiting)?
            }
            HeapNode::Object(object) => {
                self.values_contain_reachable(object.fields().values(), target, visiting)?
            }
            HeapNode::Map(map) => self.values_contain_reachable(map.values(), target, visiting)?,
            HeapNode::Interface(value) => {
                self.interface_value_contains_reachable(value, target, visiting)?
            }
            HeapNode::Exception(exception) => match exception.cause() {
                RequestExceptionCause::Local { value }
                | RequestExceptionCause::OpaqueService {
                    local_value: Some(value),
                    ..
                } => self.value_contains_reachable(value.value(), target, visiting)?,
                RequestExceptionCause::OpaqueService {
                    local_value: None, ..
                } => false,
            },
        };
        visiting.remove(&start);
        Ok(reachable)
    }

    fn interface_value_contains_reachable(
        &self,
        value: &InterfaceValue,
        target: HeapHandle,
        visiting: &mut HashSet<HeapHandle>,
    ) -> Result<bool> {
        match value.carrier() {
            InterfaceCarrier::Local { payload, .. } => {
                self.value_contains_reachable(payload, target, visiting)
            }
            InterfaceCarrier::CallbackCapability(_) => Ok(false),
        }
    }

    fn values_contain_reachable<'a>(
        &self,
        values: impl Iterator<Item = &'a RuntimeValue>,
        target: HeapHandle,
        visiting: &mut HashSet<HeapHandle>,
    ) -> Result<bool> {
        for value in values {
            if self.value_contains_reachable(value, target, visiting)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn value_contains_reachable(
        &self,
        value: &RuntimeValue,
        target: HeapHandle,
        visiting: &mut HashSet<HeapHandle>,
    ) -> Result<bool> {
        match value {
            RuntimeValue::Heap(handle) => self.is_reachable_inner(*handle, target, visiting),
            _ => Ok(false),
        }
    }

    fn set_collection_identity(
        &mut self,
        handle: HeapHandle,
        key: CollectionIdentityKey,
        identity: Option<CatchIdentity>,
    ) -> Result<()> {
        let slot = self.slot_mut(handle)?;
        match key {
            CollectionIdentityKey::Object(field) => match &mut slot.carriers {
                HeapCarrierLayout::Object(identities) => {
                    identities.insert(field, identity);
                }
                carriers => {
                    let mut identities = BTreeMap::new();
                    identities.insert(field, identity);
                    *carriers = HeapCarrierLayout::Object(identities);
                }
            },
            CollectionIdentityKey::Map(key) => match &mut slot.carriers {
                HeapCarrierLayout::Map(identities) => {
                    identities.insert(key, identity);
                }
                carriers => {
                    let mut identities = BTreeMap::new();
                    identities.insert(key, identity);
                    *carriers = HeapCarrierLayout::Map(identities);
                }
            },
        }
        Ok(())
    }

    fn slot(&self, handle: HeapHandle) -> Result<&HeapSlot> {
        if handle.epoch() != self.epoch {
            return Err(invalid_handle_error(
                handle,
                "epoch does not match heap slot",
            ));
        }
        let index = handle.index() as usize;
        let Some(slot) = self.nodes.get(index) else {
            return Err(invalid_handle_error(handle, "index is out of bounds"));
        };
        if slot.generation != handle.generation() {
            return Err(invalid_handle_error(
                handle,
                "generation does not match heap slot",
            ));
        }
        Ok(slot)
    }

    fn slot_mut(&mut self, handle: HeapHandle) -> Result<&mut HeapSlot> {
        if handle.epoch() != self.epoch {
            return Err(invalid_handle_error(
                handle,
                "epoch does not match heap slot",
            ));
        }
        let index = handle.index() as usize;
        let Some(slot) = self.nodes.get_mut(index) else {
            return Err(invalid_handle_error(handle, "index is out of bounds"));
        };
        if slot.generation != handle.generation() {
            return Err(invalid_handle_error(
                handle,
                "generation does not match heap slot",
            ));
        }
        Ok(slot)
    }
}

pub fn deep_clone_runtime_value(
    heap: &mut RequestHeap,
    value: &RuntimeValue,
) -> Result<RuntimeValue> {
    let mut context = CloneContext::default();
    let cloned = context.clone_value(heap, value, 0)?;
    heap.record_clone_depth(context.max_depth)?;
    Ok(cloned)
}

pub fn deep_clone_runtime_value_carrier(
    heap: &mut RequestHeap,
    carrier: &RuntimeValueCarrier,
) -> Result<RuntimeValueCarrier> {
    let value = deep_clone_runtime_value(heap, carrier.value())?;
    Ok(RuntimeValueCarrier::from_parts(
        value,
        carrier.catch_identity().cloned(),
    ))
}

pub fn deep_clone_runtime_value_between_heaps(
    source: &RequestHeap,
    dest: &mut RequestHeap,
    value: &RuntimeValue,
) -> Result<RuntimeValue> {
    let mut cloned =
        deep_clone_runtime_values_between_heaps(source, dest, std::slice::from_ref(value))?;
    Ok(cloned
        .pop()
        .expect("single-root cross-heap clone must return one value"))
}

/// Clones a set of roots as one graph from `source` into `dest`.
///
/// A single clone context is shared by every root, so aliases between roots
/// remain aliases in the destination heap. Only nodes reachable from at least
/// one root are copied.
pub fn deep_clone_runtime_values_between_heaps(
    source: &RequestHeap,
    dest: &mut RequestHeap,
    values: &[RuntimeValue],
) -> Result<Vec<RuntimeValue>> {
    let checkpoint = dest.checkpoint();
    let mut context = CrossHeapCloneContext::default();
    let result = values
        .iter()
        .map(|value| context.clone_value(source, dest, value, 0))
        .collect::<Result<Vec<_>>>()
        .and_then(|cloned| {
            dest.record_clone_depth(context.max_depth)?;
            Ok(cloned)
        });
    if result.is_err() {
        // Cross-heap cloning only appends destination nodes. No cloned handle
        // escapes before this function returns, so rolling the private suffix
        // back is safe and restores both length and accounting atomically.
        dest.rollback_to_checkpoint(checkpoint);
    }
    result
}

pub fn deep_clone_runtime_value_carrier_between_heaps(
    source: &RequestHeap,
    dest: &mut RequestHeap,
    carrier: &RuntimeValueCarrier,
) -> Result<RuntimeValueCarrier> {
    let value = deep_clone_runtime_value_between_heaps(source, dest, carrier.value())?;
    Ok(RuntimeValueCarrier::from_parts(
        value,
        carrier.catch_identity().cloned(),
    ))
}

/// Clones multiple carrier roots as one graph from `source` into `dest`.
///
/// A single cross-heap clone context is shared by every root, so aliases that
/// cross root boundaries remain aliases in the destination heap. Catch
/// identities remain attached to their corresponding carrier roots.
pub fn deep_clone_runtime_value_carriers_between_heaps(
    source: &RequestHeap,
    dest: &mut RequestHeap,
    carriers: &[RuntimeValueCarrier],
) -> Result<Vec<RuntimeValueCarrier>> {
    let values = carriers
        .iter()
        .map(|carrier| carrier.value().clone())
        .collect::<Vec<_>>();
    let cloned = deep_clone_runtime_values_between_heaps(source, dest, &values)?;
    Ok(cloned
        .into_iter()
        .zip(carriers)
        .map(|(value, carrier)| {
            RuntimeValueCarrier::from_parts(value, carrier.catch_identity().cloned())
        })
        .collect())
}

struct RollbackRebaseBuilder<'a> {
    source: &'a RequestHeap,
    checkpoint: RequestHeapCheckpoint,
    mapped: HashMap<HeapHandle, HeapHandle>,
    suffix_sources: Vec<HeapHandle>,
    scheduled: HashSet<HeapHandle>,
    queue: VecDeque<(HeapHandle, usize)>,
    max_depth: usize,
}

impl<'a> RollbackRebaseBuilder<'a> {
    fn new(source: &'a RequestHeap, checkpoint: RequestHeapCheckpoint) -> Self {
        let mapped = source.nodes[..checkpoint.len]
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let handle =
                    HeapHandle::new_with_epoch(index as u32, slot.generation, source.epoch);
                (handle, handle)
            })
            .collect();
        Self {
            source,
            checkpoint,
            mapped,
            suffix_sources: Vec::new(),
            scheduled: HashSet::new(),
            queue: VecDeque::new(),
            max_depth: 0,
        }
    }

    fn discover(&mut self, explicit_roots: &[RuntimeValue]) -> Result<()> {
        for index in 0..self.checkpoint.len {
            let slot = &self.source.nodes[index];
            self.schedule(
                HeapHandle::new_with_epoch(index as u32, slot.generation, self.source.epoch),
                0,
            )?;
        }
        for root in explicit_roots {
            if let RuntimeValue::Heap(handle) = root {
                self.schedule(*handle, 0)?;
            }
        }

        while let Some((handle, depth)) = self.queue.pop_front() {
            self.max_depth = self.max_depth.max(depth);
            let slot = self.source.slot(handle)?;
            for child in heap_node_child_handles(&slot.node) {
                self.schedule(child, depth.saturating_add(1))?;
            }
        }
        Ok(())
    }

    fn schedule(&mut self, handle: HeapHandle, depth: usize) -> Result<()> {
        self.source.slot(handle)?;
        if !self.scheduled.insert(handle) {
            return Ok(());
        }
        if depth > self.source.limits.max_clone_depth {
            return Err(resource_limit_error(
                "max clone depth",
                self.source.limits.max_clone_depth,
                self.max_depth,
                depth.saturating_sub(self.max_depth),
            ));
        }

        if !self.mapped.contains_key(&handle) {
            let destination_index = self
                .checkpoint
                .len
                .checked_add(self.suffix_sources.len())
                .ok_or_else(|| {
                    resource_limit_error(
                        "heap handle index space",
                        u32::MAX as usize,
                        self.checkpoint.len,
                        self.suffix_sources.len(),
                    )
                })?;
            if destination_index >= self.source.limits.max_nodes {
                return Err(resource_limit_error(
                    "max heap nodes",
                    self.source.limits.max_nodes,
                    destination_index,
                    1,
                ));
            }
            if destination_index >= u32::MAX as usize {
                return Err(resource_limit_error(
                    "heap handle index space",
                    u32::MAX as usize,
                    destination_index,
                    1,
                ));
            }
            let generation = match self.source.nodes.get(destination_index) {
                Some(slot) => slot.generation.checked_add(1).ok_or_else(|| {
                    resource_limit_error(
                        "heap slot generation space",
                        u32::MAX as usize,
                        slot.generation as usize,
                        1,
                    )
                })?,
                None => INITIAL_GENERATION,
            };
            let destination =
                HeapHandle::new_with_epoch(destination_index as u32, generation, self.source.epoch);
            self.mapped.insert(handle, destination);
            self.suffix_sources.push(handle);
        }
        self.queue.push_back((handle, depth));
        Ok(())
    }

    fn prepare(self, explicit_roots: &[RuntimeValue]) -> Result<PreparedRequestHeapRollback> {
        let mut nodes = Vec::with_capacity(
            self.checkpoint
                .len
                .saturating_add(self.suffix_sources.len()),
        );
        for slot in &self.source.nodes[..self.checkpoint.len] {
            nodes.push(rewrite_heap_slot(slot, &self.mapped)?);
        }
        for source_handle in &self.suffix_sources {
            let source_slot = self.source.slot(*source_handle)?;
            let destination = self.mapped[source_handle];
            let mut slot = rewrite_heap_slot(source_slot, &self.mapped)?;
            slot.generation = destination.generation();
            nodes.push(slot);
        }

        let estimated_bytes = nodes
            .iter()
            .map(|slot| slot.estimated_bytes)
            .fold(0usize, usize::saturating_add);
        if estimated_bytes > self.source.limits.max_estimated_bytes {
            return Err(resource_limit_error(
                "max estimated heap bytes",
                self.source.limits.max_estimated_bytes,
                0,
                estimated_bytes,
            ));
        }
        let mut stats = self.checkpoint.stats;
        stats.node_count = nodes.len();
        stats.estimated_bytes = estimated_bytes;
        stats.clone_depth = stats.clone_depth.max(self.max_depth);

        let rebased_roots = explicit_roots
            .iter()
            .map(|root| rewrite_runtime_value(root, &self.mapped))
            .collect::<Result<Vec<_>>>()?;

        Ok(PreparedRequestHeapRollback {
            nodes,
            stats,
            rebased_roots,
        })
    }
}

fn heap_node_child_handles(node: &HeapNode) -> Vec<HeapHandle> {
    let values: Vec<&RuntimeValue> = match node {
        HeapNode::Bytes(_) => Vec::new(),
        HeapNode::Array(items) => items.iter().collect(),
        HeapNode::Object(object) => object.fields().values().collect(),
        HeapNode::Map(map) => map.values().collect(),
        HeapNode::Interface(value) => match value.carrier() {
            InterfaceCarrier::Local { payload, .. } => vec![payload],
            InterfaceCarrier::CallbackCapability(_) => Vec::new(),
        },
        HeapNode::Exception(exception) => exception
            .local_value()
            .map(|value| vec![value.value()])
            .unwrap_or_default(),
    };
    values
        .into_iter()
        .filter_map(RuntimeValue::as_heap_handle)
        .collect()
}

fn rewrite_heap_slot(
    source: &HeapSlot,
    mapped: &HashMap<HeapHandle, HeapHandle>,
) -> Result<HeapSlot> {
    let node = rewrite_heap_node(&source.node, mapped)?;
    Ok(HeapSlot {
        generation: source.generation,
        estimated_bytes: estimate_heap_node_bytes(&node),
        node,
        carriers: source.carriers.clone(),
    })
}

fn rewrite_heap_node(
    source: &HeapNode,
    mapped: &HashMap<HeapHandle, HeapHandle>,
) -> Result<HeapNode> {
    Ok(match source {
        HeapNode::Bytes(bytes) => HeapNode::Bytes(bytes.clone()),
        HeapNode::Array(items) => HeapNode::Array(
            items
                .iter()
                .map(|value| rewrite_runtime_value(value, mapped))
                .collect::<Result<Vec<_>>>()?,
        ),
        HeapNode::Object(object) => HeapNode::Object(
            object.clone_with_fields(
                object
                    .fields()
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), rewrite_runtime_value(value, mapped)?)))
                    .collect::<Result<RuntimeObjectFields>>()?,
            ),
        ),
        HeapNode::Map(map) => HeapNode::Map(
            map.iter()
                .map(|(key, value)| Ok((key.clone(), rewrite_runtime_value(value, mapped)?)))
                .collect::<Result<RuntimeMap>>()?,
        ),
        HeapNode::Interface(value) => HeapNode::Interface(rewrite_interface_value(value, mapped)?),
        HeapNode::Exception(exception) => {
            let exception = match exception.local_value() {
                Some(local_value) => {
                    let rewritten = rewrite_runtime_value(local_value.value(), mapped)?;
                    let identity = local_value.catch_identity().cloned();
                    exception
                        .clone()
                        .map_local_value(|_| RuntimeValueCarrier::from_parts(rewritten, identity))
                }
                None => exception.clone(),
            };
            HeapNode::Exception(exception)
        }
    })
}

fn rewrite_interface_value(
    source: &InterfaceValue,
    mapped: &HashMap<HeapHandle, HeapHandle>,
) -> Result<InterfaceValue> {
    let carrier = match source.carrier() {
        InterfaceCarrier::Local {
            concrete_type,
            method_table,
            payload,
        } => InterfaceCarrier::Local {
            concrete_type: concrete_type.clone(),
            method_table: method_table.clone(),
            payload: rewrite_runtime_value(payload, mapped)?,
        },
        InterfaceCarrier::CallbackCapability(capability) => {
            InterfaceCarrier::CallbackCapability(capability.clone())
        }
    };
    Ok(InterfaceValue::new(source.interface().to_string(), carrier))
}

fn rewrite_runtime_value(
    source: &RuntimeValue,
    mapped: &HashMap<HeapHandle, HeapHandle>,
) -> Result<RuntimeValue> {
    let RuntimeValue::Heap(handle) = source else {
        return Ok(source.clone());
    };
    mapped
        .get(handle)
        .copied()
        .map(RuntimeValue::Heap)
        .ok_or_else(|| {
            RuntimeError::Decode(format!(
                "heap handle {handle} was not discovered during rollback rebase"
            ))
        })
}

#[derive(Default)]
struct CloneContext {
    cloned: HashMap<HeapHandle, HeapHandle>,
    active: HashSet<HeapHandle>,
    max_depth: usize,
}

impl CloneContext {
    fn clone_value(
        &mut self,
        heap: &mut RequestHeap,
        value: &RuntimeValue,
        depth: usize,
    ) -> Result<RuntimeValue> {
        self.max_depth = self.max_depth.max(depth);
        if depth > heap.limits.max_clone_depth {
            return Err(resource_limit_error(
                "max clone depth",
                heap.limits.max_clone_depth,
                self.max_depth,
                depth.saturating_sub(self.max_depth),
            ));
        }
        let RuntimeValue::Heap(handle) = value else {
            return Ok(value.clone());
        };
        self.clone_handle(heap, *handle, depth)
            .map(RuntimeValue::Heap)
    }

    fn clone_handle(
        &mut self,
        heap: &mut RequestHeap,
        handle: HeapHandle,
        depth: usize,
    ) -> Result<HeapHandle> {
        if let Some(cloned) = self.cloned.get(&handle) {
            return Ok(*cloned);
        }
        if !self.active.insert(handle) {
            return Err(RuntimeError::Decode(format!(
                "cannot clone cyclic heap graph at handle {handle}"
            )));
        }

        let slot = heap.slot(handle)?.clone();
        let carriers = slot.carriers;
        let node = slot.node;
        let cloned_node = match node {
            HeapNode::Bytes(bytes) => HeapNode::Bytes(bytes),
            HeapNode::Array(items) => {
                let cloned_items = items
                    .iter()
                    .map(|value| self.clone_value(heap, value, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                HeapNode::Array(cloned_items)
            }
            HeapNode::Object(object) => {
                let cloned_fields = object
                    .fields()
                    .iter()
                    .map(|(key, value)| {
                        Ok((key.clone(), self.clone_value(heap, value, depth + 1)?))
                    })
                    .collect::<Result<RuntimeObjectFields>>()?;
                HeapNode::Object(object.clone_with_fields(cloned_fields))
            }
            HeapNode::Map(map) => {
                let cloned_map = map
                    .iter()
                    .map(|(key, value)| {
                        Ok((key.clone(), self.clone_value(heap, value, depth + 1)?))
                    })
                    .collect::<Result<RuntimeMap>>()?;
                HeapNode::Map(cloned_map)
            }
            HeapNode::Interface(value) => {
                HeapNode::Interface(self.clone_interface_value(heap, &value, depth + 1)?)
            }
            HeapNode::Exception(exception) => {
                let cloned = match exception.cause() {
                    RequestExceptionCause::Local { value }
                    | RequestExceptionCause::OpaqueService {
                        local_value: Some(value),
                        ..
                    } => {
                        let cloned_value = self.clone_value(heap, value.value(), depth + 1)?;
                        let identity = value.catch_identity().cloned();
                        exception.map_local_value(|_| {
                            RuntimeValueCarrier::from_parts(cloned_value, identity)
                        })
                    }
                    RequestExceptionCause::OpaqueService {
                        local_value: None, ..
                    } => exception,
                };
                HeapNode::Exception(cloned)
            }
        };

        let cloned_handle = heap.alloc_node_with_carriers(cloned_node, carriers)?;
        self.active.remove(&handle);
        self.cloned.insert(handle, cloned_handle);
        Ok(cloned_handle)
    }

    fn clone_interface_value(
        &mut self,
        heap: &mut RequestHeap,
        value: &InterfaceValue,
        depth: usize,
    ) -> Result<InterfaceValue> {
        let carrier = match value.carrier() {
            InterfaceCarrier::Local {
                concrete_type,
                method_table,
                payload,
            } => InterfaceCarrier::Local {
                concrete_type: concrete_type.clone(),
                method_table: method_table.clone(),
                payload: self.clone_value(heap, payload, depth)?,
            },
            InterfaceCarrier::CallbackCapability(capability) => {
                InterfaceCarrier::CallbackCapability(capability.clone())
            }
        };
        Ok(InterfaceValue::new(value.interface().to_string(), carrier))
    }
}

#[derive(Default)]
struct CrossHeapCloneContext {
    cloned: HashMap<HeapHandle, HeapHandle>,
    active: HashSet<HeapHandle>,
    max_depth: usize,
}

impl CrossHeapCloneContext {
    fn clone_value(
        &mut self,
        source: &RequestHeap,
        dest: &mut RequestHeap,
        value: &RuntimeValue,
        depth: usize,
    ) -> Result<RuntimeValue> {
        self.max_depth = self.max_depth.max(depth);
        if depth > dest.limits.max_clone_depth {
            return Err(resource_limit_error(
                "max clone depth",
                dest.limits.max_clone_depth,
                self.max_depth,
                depth.saturating_sub(self.max_depth),
            ));
        }
        let RuntimeValue::Heap(handle) = value else {
            return Ok(value.clone());
        };
        self.clone_handle(source, dest, *handle, depth)
            .map(RuntimeValue::Heap)
    }

    fn clone_handle(
        &mut self,
        source: &RequestHeap,
        dest: &mut RequestHeap,
        handle: HeapHandle,
        depth: usize,
    ) -> Result<HeapHandle> {
        if let Some(cloned) = self.cloned.get(&handle) {
            return Ok(*cloned);
        }
        if !self.active.insert(handle) {
            return Err(RuntimeError::Decode(format!(
                "cannot clone cyclic heap graph at handle {handle}"
            )));
        }

        let slot = source.slot(handle)?.clone();
        let carriers = slot.carriers;
        let node = slot.node;
        let cloned_node = match node {
            HeapNode::Bytes(bytes) => HeapNode::Bytes(bytes),
            HeapNode::Array(items) => {
                let cloned_items = items
                    .iter()
                    .map(|value| self.clone_value(source, dest, value, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                HeapNode::Array(cloned_items)
            }
            HeapNode::Object(object) => {
                let cloned_fields = object
                    .fields()
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            key.clone(),
                            self.clone_value(source, dest, value, depth + 1)?,
                        ))
                    })
                    .collect::<Result<RuntimeObjectFields>>()?;
                HeapNode::Object(object.clone_with_fields(cloned_fields))
            }
            HeapNode::Map(map) => {
                let cloned_map = map
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            key.clone(),
                            self.clone_value(source, dest, value, depth + 1)?,
                        ))
                    })
                    .collect::<Result<RuntimeMap>>()?;
                HeapNode::Map(cloned_map)
            }
            HeapNode::Interface(value) => {
                HeapNode::Interface(self.clone_interface_value(source, dest, &value, depth + 1)?)
            }
            HeapNode::Exception(exception) => {
                let cloned = match exception.cause() {
                    RequestExceptionCause::Local { value }
                    | RequestExceptionCause::OpaqueService {
                        local_value: Some(value),
                        ..
                    } => {
                        let cloned_value =
                            self.clone_value(source, dest, value.value(), depth + 1)?;
                        let identity = value.catch_identity().cloned();
                        exception.map_local_value(|_| {
                            RuntimeValueCarrier::from_parts(cloned_value, identity)
                        })
                    }
                    RequestExceptionCause::OpaqueService {
                        local_value: None, ..
                    } => exception,
                };
                HeapNode::Exception(cloned)
            }
        };

        let cloned_handle = dest.alloc_node_with_carriers(cloned_node, carriers)?;
        self.active.remove(&handle);
        self.cloned.insert(handle, cloned_handle);
        Ok(cloned_handle)
    }

    fn clone_interface_value(
        &mut self,
        source: &RequestHeap,
        dest: &mut RequestHeap,
        value: &InterfaceValue,
        depth: usize,
    ) -> Result<InterfaceValue> {
        let carrier = match value.carrier() {
            InterfaceCarrier::Local {
                concrete_type,
                method_table,
                payload,
            } => InterfaceCarrier::Local {
                concrete_type: concrete_type.clone(),
                method_table: method_table.clone(),
                payload: self.clone_value(source, dest, payload, depth)?,
            },
            InterfaceCarrier::CallbackCapability(capability) => {
                InterfaceCarrier::CallbackCapability(capability.clone())
            }
        };
        Ok(InterfaceValue::new(value.interface().to_string(), carrier))
    }
}

impl Default for RequestHeap {
    fn default() -> Self {
        Self::new(RequestHeapLimits::default())
    }
}

#[derive(Clone, Debug)]
struct HeapSlot {
    generation: u32,
    #[allow(dead_code)]
    estimated_bytes: usize,
    node: HeapNode,
    carriers: HeapCarrierLayout,
}

#[derive(Clone, Debug)]
enum HeapCarrierLayout {
    None,
    Array(Vec<Option<CatchIdentity>>),
    Object(BTreeMap<String, Option<CatchIdentity>>),
    Map(BTreeMap<RuntimeValueKey, Option<CatchIdentity>>),
    Interface(Option<CatchIdentity>),
    LocalCarrierCell(Option<CatchIdentity>),
}

impl HeapCarrierLayout {
    fn for_node(node: &HeapNode) -> Self {
        match node {
            HeapNode::Array(items) => Self::Array(array_identity_slots(items.len())),
            HeapNode::Object(object) => Self::Object(
                object
                    .fields()
                    .keys()
                    .cloned()
                    .map(|field| (field, None))
                    .collect(),
            ),
            HeapNode::Map(map) => Self::Map(map.keys().cloned().map(|key| (key, None)).collect()),
            HeapNode::Interface(_) => Self::Interface(None),
            HeapNode::Bytes(_) | HeapNode::Exception(_) => Self::None,
        }
    }
}

enum CollectionIdentityKey {
    Object(String),
    Map(RuntimeValueKey),
}

enum MapEntrySetPlan {
    Map {
        existed: bool,
        old_bytes: usize,
        new_bytes: usize,
    },
    ObjectField {
        field: String,
        existed: bool,
        old_bytes: usize,
        new_bytes: usize,
    },
}

enum MapEntryDeletePlan {
    Map { old_bytes: usize },
    ObjectField { field: String },
}

enum ObjectFieldSetPlan {
    Object {
        existed: bool,
        old_bytes: usize,
        new_bytes: usize,
    },
    MapEntry {
        key: RuntimeValueKey,
        existed: bool,
        old_bytes: usize,
        new_bytes: usize,
    },
}

enum ObjectFieldDeletePlan {
    Object {
        old_bytes: usize,
    },
    MapEntry {
        key: RuntimeValueKey,
        old_bytes: usize,
    },
}

fn invalid_handle_error(handle: HeapHandle, reason: &str) -> RuntimeError {
    RuntimeError::Decode(format!("invalid heap handle {handle}: {reason}"))
}

fn resource_limit_error(
    reason: &str,
    limit: usize,
    current: usize,
    requested_delta: usize,
) -> RuntimeError {
    RuntimeError::ResourceLimitExceeded {
        resource: "requestHeap".to_string(),
        reason: reason.to_string(),
        limit,
        current,
        requested_delta,
    }
}

fn object_field_key_from_map_key(key: &RuntimeValueKey) -> Result<&str> {
    match key {
        RuntimeValueKey::String(key) => Ok(key),
    }
}

fn estimate_heap_node_bytes(node: &HeapNode) -> usize {
    match node {
        HeapNode::Bytes(bytes) => NODE_OVERHEAD_BYTES.saturating_add(bytes.len()),
        HeapNode::Array(items) => {
            NODE_OVERHEAD_BYTES.saturating_add(items.iter().map(estimate_array_item_bytes).sum())
        }
        HeapNode::Object(object) => object
            .fields()
            .iter()
            .fold(NODE_OVERHEAD_BYTES, |total, (key, value)| {
                total.saturating_add(estimate_object_field_bytes(key, value))
            }),
        HeapNode::Map(map) => map.iter().fold(NODE_OVERHEAD_BYTES, |total, (key, value)| {
            total.saturating_add(estimate_map_entry_bytes(key, value))
        }),
        HeapNode::Interface(value) => estimate_interface_value_bytes(value),
        HeapNode::Exception(exception) => {
            let stack_bytes = exception.stack().len().saturating_mul(NODE_OVERHEAD_BYTES);
            NODE_OVERHEAD_BYTES
                .saturating_add(stack_bytes)
                .saturating_add(exception.correlation().trace_id.len())
                .saturating_add(exception.correlation().error_id.len())
        }
    }
}

fn split_carriers(
    carriers: Vec<RuntimeValueCarrier>,
) -> (Vec<RuntimeValue>, Vec<Option<CatchIdentity>>) {
    carriers
        .into_iter()
        .map(RuntimeValueCarrier::into_parts)
        .unzip()
}

fn array_identity_slots(len: usize) -> Vec<Option<CatchIdentity>> {
    vec![None; len]
}

fn estimate_interface_value_bytes(value: &InterfaceValue) -> usize {
    let base = NODE_OVERHEAD_BYTES
        .saturating_add(INTERFACE_VALUE_OVERHEAD_BYTES)
        .saturating_add(value.interface().len());
    match value.carrier() {
        InterfaceCarrier::Local {
            concrete_type,
            method_table,
            payload,
        } => base
            .saturating_add(concrete_type.len())
            .saturating_add(estimate_interface_method_table_bytes(method_table))
            .saturating_add(estimate_value_bytes(payload)),
        InterfaceCarrier::CallbackCapability(capability) => base
            .saturating_add(capability.owner_runtime_replica_id().len())
            .saturating_add(capability.owner_activation_id().len())
            .saturating_add(std::mem::size_of::<u64>())
            .saturating_add(capability.interface_or_adapter_contract().len())
            .saturating_add(capability.opaque_capability_id().len()),
    }
}

fn estimate_interface_method_table_bytes(table: &InterfaceMethodTable) -> usize {
    table
        .id()
        .len()
        .saturating_add(table.interface_abi_id().len())
        .saturating_add(table.slots().iter().fold(0usize, |total, slot| {
            total
                .saturating_add(INTERFACE_METHOD_SLOT_OVERHEAD_BYTES)
                .saturating_add(slot.method_abi_id().len())
                .saturating_add(estimate_interface_method_target_bytes(slot.target()))
        }))
}

fn estimate_interface_method_target_bytes(target: &InterfaceMethodTarget) -> usize {
    match target {
        InterfaceMethodTarget::LocalExecutable {
            executable,
            receiver_call_abi: _,
        } => executable.to_string().len(),
    }
}

fn apply_estimated_bytes_replacement(
    estimated_bytes: &mut usize,
    old_bytes: usize,
    new_bytes: usize,
) {
    *estimated_bytes = estimated_bytes
        .saturating_sub(old_bytes)
        .saturating_add(new_bytes);
}

fn estimate_array_item_bytes(value: &RuntimeValue) -> usize {
    ARRAY_ITEM_OVERHEAD_BYTES.saturating_add(estimate_value_bytes(value))
}

fn estimate_object_field_bytes(key: &str, value: &RuntimeValue) -> usize {
    OBJECT_FIELD_OVERHEAD_BYTES
        .saturating_add(key.len())
        .saturating_add(estimate_value_bytes(value))
}

fn estimate_map_entry_bytes(key: &RuntimeValueKey, value: &RuntimeValue) -> usize {
    MAP_ENTRY_OVERHEAD_BYTES
        .saturating_add(estimate_key_bytes(key))
        .saturating_add(estimate_value_bytes(value))
}

fn estimate_value_bytes(value: &RuntimeValue) -> usize {
    match value {
        RuntimeValue::Null => 0,
        RuntimeValue::Bool(_) => 1,
        RuntimeValue::Number(_) => 8,
        RuntimeValue::Date(_) => 8,
        RuntimeValue::String(value) => value.len(),
        RuntimeValue::ActorRef(actor_ref) => estimate_actor_ref_bytes(actor_ref),
        RuntimeValue::Heap(_) => HANDLE_BYTES,
    }
}

fn estimate_actor_ref_bytes(actor_ref: &ActorRef) -> usize {
    actor_ref
        .service_id()
        .len()
        .saturating_add(actor_ref.actor_type_identity().len())
        .saturating_add(actor_ref.actor_id_type_identity().len())
        .saturating_add(actor_ref.actor_id_encoding_version().len())
        .saturating_add(actor_ref.canonical_actor_id_key_bytes().len())
        .saturating_add(actor_ref.actor_id_hash().len())
        .saturating_add(std::mem::size_of::<Option<u64>>())
}

fn estimate_key_bytes(key: &RuntimeValueKey) -> usize {
    match key {
        RuntimeValueKey::String(value) => value.len(),
    }
}

#[cfg(test)]
mod tests;
