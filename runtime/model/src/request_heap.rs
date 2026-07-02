use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::{
    error::{Result, RuntimeModelError as RuntimeError},
    value::{
        ActorRef, HeapHandle, HeapNode, InterfaceCarrier, InterfaceMethodTable,
        InterfaceMethodTarget, InterfaceValue, RuntimeBytes, RuntimeMap, RuntimeObject,
        RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
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
pub struct RequestHeap {
    nodes: Vec<HeapSlot>,
    limits: RequestHeapLimits,
    stats: RequestHeapStats,
}

impl RequestHeap {
    pub fn new(limits: RequestHeapLimits) -> Self {
        Self {
            nodes: Vec::new(),
            limits,
            stats: RequestHeapStats::default(),
        }
    }

    pub fn limits(&self) -> &RequestHeapLimits {
        &self.limits
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

    pub fn alloc_array(&mut self, items: Vec<RuntimeValue>) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Array(items))
    }

    pub fn alloc_bytes(&mut self, bytes: impl Into<RuntimeBytes>) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Bytes(bytes.into()))
    }

    pub fn alloc_object(&mut self, object: RuntimeObject) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Object(object))
    }

    pub fn alloc_map(&mut self, map: RuntimeMap) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Map(map))
    }

    pub fn alloc_interface(&mut self, value: InterfaceValue) -> Result<HeapHandle> {
        self.alloc_node(HeapNode::Interface(value))
    }

    pub fn get(&self, handle: HeapHandle) -> Result<&HeapNode> {
        self.slot(handle).map(|slot| &slot.node)
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
        self.check_insert_without_cycle(handle, &value)?;
        self.push_array_item_inner(handle, value)
    }

    pub fn set_array_item(
        &mut self,
        handle: HeapHandle,
        index: usize,
        value: RuntimeValue,
    ) -> Result<()> {
        self.check_insert_without_cycle(handle, &value)?;
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
            apply_estimated_bytes_replacement(&mut slot.estimated_bytes, old_bytes, new_bytes);
        }
        self.apply_stats_estimated_bytes_replacement(old_bytes, new_bytes);
        Ok(())
    }

    pub fn pop_array_item(&mut self, handle: HeapHandle) -> Result<RuntimeValue> {
        let popped_bytes = {
            let slot = self.slot(handle)?;
            let HeapNode::Array(items) = &slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.pop target is not an array".to_string(),
                ));
            };
            items.last().map(estimate_array_item_bytes).unwrap_or(0)
        };

        let popped = {
            let slot = self.slot_mut(handle)?;
            let HeapNode::Array(items) = &mut slot.node else {
                return Err(RuntimeError::Decode(
                    "Array.pop target is not an array".to_string(),
                ));
            };
            let popped = items.pop().unwrap_or(RuntimeValue::Null);
            if popped_bytes > 0 {
                slot.estimated_bytes = slot.estimated_bytes.saturating_sub(popped_bytes);
            }
            popped
        };
        if popped_bytes > 0 {
            self.stats.estimated_bytes = self.stats.estimated_bytes.saturating_sub(popped_bytes);
        }
        Ok(popped)
    }

    pub fn set_map_entry(
        &mut self,
        handle: HeapHandle,
        key: RuntimeValueKey,
        value: RuntimeValue,
    ) -> Result<bool> {
        self.check_insert_without_cycle(handle, &value)?;
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
        match plan {
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
        }
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
        self.check_insert_without_cycle(handle, &value)?;
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
        match plan {
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
        }
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

        let handle = HeapHandle::new(self.nodes.len() as u32, INITIAL_GENERATION);
        self.nodes.push(HeapSlot {
            generation: INITIAL_GENERATION,
            estimated_bytes,
            node,
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

    fn push_array_item_inner(&mut self, handle: HeapHandle, value: RuntimeValue) -> Result<()> {
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
            InterfaceCarrier::Remote { .. } => Ok(false),
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

    fn slot(&self, handle: HeapHandle) -> Result<&HeapSlot> {
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

pub fn deep_clone_runtime_value_between_heaps(
    source: &RequestHeap,
    dest: &mut RequestHeap,
    value: &RuntimeValue,
) -> Result<RuntimeValue> {
    let mut context = CrossHeapCloneContext::default();
    let cloned = context.clone_value(source, dest, value, 0)?;
    dest.record_clone_depth(context.max_depth)?;
    Ok(cloned)
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

        let node = heap.get(handle)?.clone();
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
        };

        let cloned_handle = heap.alloc_node(cloned_node)?;
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
            InterfaceCarrier::Remote {
                dependency_ref,
                public_instance_key,
                operations,
            } => InterfaceCarrier::Remote {
                dependency_ref: dependency_ref.clone(),
                public_instance_key: public_instance_key.clone(),
                operations: operations.clone(),
            },
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

        let node = source.get(handle)?.clone();
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
        };

        let cloned_handle = dest.alloc_node(cloned_node)?;
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
            InterfaceCarrier::Remote {
                dependency_ref,
                public_instance_key,
                operations,
            } => InterfaceCarrier::Remote {
                dependency_ref: dependency_ref.clone(),
                public_instance_key: public_instance_key.clone(),
                operations: operations.clone(),
            },
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
    }
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
        InterfaceCarrier::Remote {
            dependency_ref,
            public_instance_key,
            operations,
        } => base
            .saturating_add(dependency_ref.len())
            .saturating_add(public_instance_key.len())
            .saturating_add(estimate_remote_operation_table_bytes(operations)),
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

fn estimate_remote_operation_table_bytes(table: &crate::value::RemoteOperationTable) -> usize {
    table
        .id()
        .len()
        .saturating_add(table.interface_abi_id().len())
        .saturating_add(table.slots().iter().fold(0usize, |total, slot| {
            total
                .saturating_add(INTERFACE_METHOD_SLOT_OVERHEAD_BYTES)
                .saturating_add(slot.method_abi_id().len())
                .saturating_add(slot.operation_abi_id().len())
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
