use super::env::Flow;
use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_program::LinkedExecutable;
use skiff_runtime_model::runtime_value::RuntimeValue;

pub struct FlowCompletionPolicy;

impl FlowCompletionPolicy {
    pub fn callable_value(flow: Flow, symbol: &str) -> Result<RuntimeValue> {
        match flow {
            Flow::Return(value) => Ok(value),
            Flow::Continue | Flow::Parked => Ok(RuntimeValue::Null),
            Flow::Break | Flow::LoopContinue => Err(RuntimeError::Decode(format!(
                "callable {symbol} exited with break/continue outside a loop"
            ))),
            Flow::ContinueConsumer => Err(RuntimeError::Decode(format!(
                "callable {symbol} exited with continue consumer outside a process consumer"
            ))),
        }
    }

    pub fn const_value(flow: Flow, name: &str) -> Result<RuntimeValue> {
        match flow {
            Flow::Return(value) => Ok(value),
            Flow::Continue | Flow::Parked => Ok(RuntimeValue::Null),
            Flow::Break | Flow::LoopContinue => Err(RuntimeError::Decode(format!(
                "const {name} exited with break/continue outside a loop"
            ))),
            Flow::ContinueConsumer => Err(RuntimeError::Decode(format!(
                "const {name} exited with continue consumer outside a process consumer"
            ))),
        }
    }

    pub fn value_block_value(flow: Flow) -> Result<Option<RuntimeValue>> {
        match flow {
            Flow::Continue => Ok(None),
            Flow::Return(value) => Ok(Some(value)),
            Flow::Parked => Err(RuntimeError::Decode(
                "valueBlock exited with parked wait".to_string(),
            )),
            Flow::Break | Flow::LoopContinue => Err(RuntimeError::Decode(
                "valueBlock exited with break/continue outside a loop".to_string(),
            )),
            Flow::ContinueConsumer => Err(RuntimeError::Decode(
                "valueBlock exited with continue consumer".to_string(),
            )),
        }
    }

    pub fn non_returning_expression_value(flow: Flow, label: &str) -> Result<RuntimeValue> {
        match flow {
            Flow::Continue => Ok(RuntimeValue::Null),
            Flow::Parked => Err(RuntimeError::Decode(format!(
                "{label} expression exited with parked wait"
            ))),
            Flow::Return(_) => Err(RuntimeError::Decode(format!(
                "{label} expression returned unexpectedly"
            ))),
            Flow::Break | Flow::LoopContinue => Err(RuntimeError::Decode(format!(
                "{label} expression exited with break/continue"
            ))),
            Flow::ContinueConsumer => Err(RuntimeError::Decode(format!(
                "{label} expression exited with continue consumer"
            ))),
        }
    }

    pub fn entry_loop_control_error(executable: &LinkedExecutable) -> RuntimeError {
        RuntimeError::Decode(format!(
            "executable {} exited with break/continue outside a loop",
            executable.symbol
        ))
    }

    pub fn entry_consumer_error(executable: &LinkedExecutable) -> RuntimeError {
        RuntimeError::Decode(format!(
            "executable {} exited with continue consumer outside a process consumer",
            executable.symbol
        ))
    }
}
