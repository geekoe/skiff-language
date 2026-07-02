use crate::ExecutionControl;

#[derive(Clone)]
pub struct TimeCapabilityContext<'a> {
    execution_control: ExecutionControl<'a>,
}

impl<'a> TimeCapabilityContext<'a> {
    pub fn new(execution_control: ExecutionControl<'a>) -> Self {
        Self { execution_control }
    }

    pub fn execution_control(&self) -> ExecutionControl<'a> {
        self.execution_control.clone()
    }
}
