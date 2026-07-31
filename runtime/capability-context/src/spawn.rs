use crate::{
    ActorCapabilityContext, CapabilityResult, OwnedExecutionControl, SpawnSubmitControlRequest,
};

/// Concrete eval-side client for cross-assembly `spawn` statement submission.
pub struct SpawnClient<'a> {
    context: ActorCapabilityContext<'a>,
}

impl<'a> SpawnClient<'a> {
    pub fn new(context: ActorCapabilityContext<'a>) -> Self {
        Self { context }
    }

    pub async fn submit_spawn(
        &self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityResult<()> {
        self.context
            .submit_spawn(request, args_payload, execution_control)
            .await
    }
}
