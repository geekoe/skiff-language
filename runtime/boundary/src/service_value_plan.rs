mod compile;
mod matcher;

use serde_json::Value;
use skiff_artifact_model::ContractTypeRef;
use skiff_runtime_model::{
    request_heap::RequestHeap, type_plan::RuntimeTypePlan, value::RuntimeValue,
};

use crate::{
    binary::{decode_payload_plan, encode_payload_plan},
    json_convert::{decode_wire_plan_impl, encode_wire_plan_impl, BoundaryStreamHandlePolicy},
    package_schema_records::PackageSchemaRecords,
    payload::PayloadBoundary,
    service_linkable::ServiceLinkableMaterializationError,
};

/// The one expected-type plan for an ordinary value crossing a service boundary.
///
/// The plan is compiled only from a pinned `ContractTypeRef`, its owning boundary schema and the
/// canonical HTTP/WebSocket shape owners. Runtime/executable descriptors are deliberately absent.
/// The detached matcher and both canonical codecs consume the same compiled `RuntimeTypePlan`.
pub struct ServiceValuePlan<'contract> {
    contract_type: &'contract ContractTypeRef,
    runtime_type: RuntimeTypePlan,
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

    #[cfg(test)]
    pub(crate) fn runtime_type_plan(&self) -> &RuntimeTypePlan {
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

pub(super) fn codec_error(error: crate::RuntimeError) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::Codec {
        message: error.to_string(),
    }
}
