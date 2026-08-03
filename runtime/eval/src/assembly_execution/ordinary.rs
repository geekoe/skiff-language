use skiff_runtime_linked_program::{CallIr, LinkedPackageDirectCall};
use skiff_runtime_model::runtime_value::RuntimeValueCarrier;

use crate::{error::Result, eval_context::EvalContext};

pub(crate) async fn execute_package_direct(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: &LinkedPackageDirectCall,
    args: Vec<RuntimeValueCarrier>,
) -> Result<RuntimeValueCarrier> {
    let call_context = context
        .context
        .clone()
        .with_local_call_site(call.site.clone());
    if let Some(receiver_const) = target.receiver_const() {
        let receiver = context
            .interpreter
            .eval_program_const_addr(
                context.context.clone(),
                context.heap,
                context.env,
                receiver_const,
            )
            .await?;
        context
            .interpreter
            .call_program_executable_with_self_carriers(
                call_context,
                context.heap,
                context.env,
                context.addr,
                target.executable_addr(),
                &call.type_args,
                receiver,
                args,
            )
            .await
    } else {
        context
            .interpreter
            .call_program_executable_carriers(
                call_context,
                context.heap,
                context.env,
                context.addr,
                target.executable_addr(),
                &call.type_args,
                args,
            )
            .await
    }
}

#[cfg(test)]
pub(crate) mod tests;
