mod compile;
mod matcher;

use serde_json::Value;
use skiff_artifact_model::ContractTypeRef;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
    value::RuntimeValue,
};

use crate::{
    binary::{decode_payload_plan, encode_payload_plan},
    json_convert::{decode_wire_plan_impl, encode_wire_plan_impl, BoundaryStreamHandlePolicy},
    package_schema_records::PackageSchemaRecords,
    payload::PayloadBoundary,
    service_linkable::ServiceLinkableMaterializationError,
};

const ROOT_UNION_ORDINAL_OFFSET: usize = 5;

/// The one expected-type plan for an ordinary value crossing a service boundary.
///
/// The plan is compiled only from a pinned `ContractTypeRef`, its owning boundary schema and the
/// canonical HTTP/WebSocket shape owners. Runtime/executable descriptors are deliberately absent.
/// The detached matcher and both canonical codecs consume the same compiled `RuntimeTypePlan`.
pub struct ServiceValuePlan<'contract> {
    contract_type: &'contract ContractTypeRef,
    runtime_type: RuntimeTypePlan,
}

/// The exact compiled root selected for one canonical service value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceValueSelection {
    /// The compiled root plan itself.
    Root,
    /// One exact branch of a compiled named-union root.
    NamedUnionBranch(usize),
}

/// A decoded canonical service value together with its binary root selection.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedSelectedServiceValue {
    /// The decoded runtime value allocated in the destination request heap.
    pub value: RuntimeValue,
    /// The root or exact root-union branch selected by the binary payload.
    pub selection: ServiceValueSelection,
}

impl<'contract> ServiceValuePlan<'contract> {
    pub fn compile(
        contract_type: &'contract ContractTypeRef,
        package_schema_records: &PackageSchemaRecords,
    ) -> Result<Self, ServiceLinkableMaterializationError> {
        let runtime_type = compile::compile(contract_type, package_schema_records)?;
        Ok(Self {
            contract_type,
            runtime_type,
        })
    }

    pub const fn contract_type(&self) -> &ContractTypeRef {
        self.contract_type
    }

    pub fn runtime_type_plan(&self) -> &RuntimeTypePlan {
        &self.runtime_type
    }

    pub fn value_matches(
        &self,
        value: &RuntimeValue,
        heap: &RequestHeap,
    ) -> Result<bool, ServiceLinkableMaterializationError> {
        matcher::value_matches(value, heap, &self.runtime_type)
    }

    pub fn validate_value(
        &self,
        value: &RuntimeValue,
        heap: &RequestHeap,
    ) -> Result<(), ServiceLinkableMaterializationError> {
        if self.value_matches(value, heap)? {
            Ok(())
        } else {
            Err(ServiceLinkableMaterializationError::TypeMismatch)
        }
    }

    pub fn encode_binary(
        &self,
        value: &RuntimeValue,
        boundary: &PayloadBoundary,
        heap: &RequestHeap,
    ) -> Result<Vec<u8>, ServiceLinkableMaterializationError> {
        self.validate_value(value, heap)?;
        encode_payload_plan(value, &self.runtime_type, boundary, heap).map_err(codec_error)
    }

    pub fn decode_binary(
        &self,
        bytes: &[u8],
        boundary: &PayloadBoundary,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue, ServiceLinkableMaterializationError> {
        let checkpoint = heap.checkpoint();
        let result = decode_payload_plan(bytes, &self.runtime_type, boundary, heap)
            .map_err(codec_error)
            .and_then(|value| {
                self.validate_value(&value, heap)?;
                Ok(value)
            });
        if result.is_err() {
            heap.rollback_to_checkpoint(checkpoint);
        }
        result
    }

    /// Encodes only the selected compiled root plan.
    ///
    /// Root unions require an exact branch ordinal; non-union roots reject branch selections.
    pub fn encode_binary_selected(
        &self,
        value: &RuntimeValue,
        selection: ServiceValueSelection,
        boundary: &PayloadBoundary,
        heap: &RequestHeap,
    ) -> Result<Vec<u8>, ServiceLinkableMaterializationError> {
        match (self.runtime_type.node(), selection) {
            (RuntimeTypeNode::Union(_), ServiceValueSelection::Root) => Err(selection_error(
                "named-union service value requires an explicit branch selection",
            )),
            (RuntimeTypeNode::Union(branches), ServiceValueSelection::NamedUnionBranch(index)) => {
                let branch = selected_union_branch(branches, index)?;
                validate_plan_value(value, heap, branch)?;
                let mut bytes =
                    encode_payload_plan(value, branch, boundary, heap).map_err(codec_error)?;
                let ordinal = u8::try_from(index)
                    .map_err(|_| selection_error("named-union branch selection is out of range"))?;
                bytes.insert(ROOT_UNION_ORDINAL_OFFSET, ordinal);
                Ok(bytes)
            }
            (_, ServiceValueSelection::Root) => self.encode_binary(value, boundary, heap),
            (_, ServiceValueSelection::NamedUnionBranch(_)) => Err(selection_error(
                "non-union service value only accepts the root selection",
            )),
        }
    }

    /// Decodes a binary service value and preserves its root union ordinal.
    ///
    /// Nested union ordinals remain an implementation detail of their recursive payload plans.
    pub fn decode_binary_selected(
        &self,
        bytes: &[u8],
        boundary: &PayloadBoundary,
        heap: &mut RequestHeap,
    ) -> Result<DecodedSelectedServiceValue, ServiceLinkableMaterializationError> {
        let RuntimeTypeNode::Union(branches) = self.runtime_type.node() else {
            return self.decode_binary(bytes, boundary, heap).map(|value| {
                DecodedSelectedServiceValue {
                    value,
                    selection: ServiceValueSelection::Root,
                }
            });
        };
        validate_union_branch_count(branches)?;

        let checkpoint = heap.checkpoint();
        let result = decode_payload_plan(bytes, &self.runtime_type, boundary, heap)
            .map_err(codec_error)
            .and_then(|value| {
                let index = bytes
                    .get(ROOT_UNION_ORDINAL_OFFSET)
                    .copied()
                    .map(usize::from)
                    .ok_or_else(|| selection_error("named-union payload has no branch ordinal"))?;
                let branch = selected_union_branch(branches, index)?;
                validate_plan_value(&value, heap, branch)?;
                Ok(DecodedSelectedServiceValue {
                    value,
                    selection: ServiceValueSelection::NamedUnionBranch(index),
                })
            });
        if result.is_err() {
            heap.rollback_to_checkpoint(checkpoint);
        }
        result
    }

    pub fn encode_json_value(
        &self,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<Value, ServiceLinkableMaterializationError> {
        self.validate_value(value, heap)?;
        encode_wire_plan_impl(
            value,
            &self.runtime_type,
            heap,
            BoundaryStreamHandlePolicy::ExternalBoundary,
        )
        .map_err(codec_error)
    }

    pub fn decode_json_value(
        &self,
        value: &Value,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue, ServiceLinkableMaterializationError> {
        let checkpoint = heap.checkpoint();
        let result = decode_wire_plan_impl(
            value,
            &self.runtime_type,
            heap,
            BoundaryStreamHandlePolicy::ExternalBoundary,
        )
        .map_err(codec_error)
        .and_then(|value| {
            self.validate_value(&value, heap)?;
            Ok(value)
        });
        if result.is_err() {
            heap.rollback_to_checkpoint(checkpoint);
        }
        result
    }
}

fn selected_union_branch(
    branches: &[RuntimeTypePlan],
    index: usize,
) -> Result<&RuntimeTypePlan, ServiceLinkableMaterializationError> {
    validate_union_branch_count(branches)?;
    branches
        .get(index)
        .ok_or_else(|| selection_error("named-union branch selection is out of range"))
}

fn validate_union_branch_count(
    branches: &[RuntimeTypePlan],
) -> Result<(), ServiceLinkableMaterializationError> {
    if branches.len() > u8::MAX as usize + 1 {
        Err(selection_error(
            "named-union branch count exceeds the binary ordinal range",
        ))
    } else {
        Ok(())
    }
}

fn validate_plan_value(
    value: &RuntimeValue,
    heap: &RequestHeap,
    plan: &RuntimeTypePlan,
) -> Result<(), ServiceLinkableMaterializationError> {
    if matcher::value_matches(value, heap, plan)? {
        Ok(())
    } else {
        Err(ServiceLinkableMaterializationError::TypeMismatch)
    }
}

fn selection_error(message: impl Into<String>) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::Codec {
        message: message.into(),
    }
}

pub(super) fn codec_error(error: crate::RuntimeError) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::Codec {
        message: error.to_string(),
    }
}
