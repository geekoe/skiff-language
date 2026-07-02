use super::{unsupported_native_target, RuntimeNativeInvocation};
use crate::capability::NativeTimeCapability;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};

const TIME_SLEEP_KEY: &str = "std.time.sleep";
pub(super) const TIME_SLEEP_MAX_MILLIS: u64 = 60_000;
const TIME_SLEEP_POLL_MILLIS: u64 = 10;

pub(super) struct TimeNativeDispatch;

impl TimeNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        target == TIME_SLEEP_KEY
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch<TimeContext>(
        time_context: &TimeContext,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        TimeContext: NativeTimeCapability,
    {
        let binding_key = invocation.binding_key();
        let native_boundary = invocation.native_boundary()?;
        match binding_key {
            TIME_SLEEP_KEY => {
                let value = args.first().ok_or_else(|| {
                    RuntimeError::Decode(format!("{diagnostic_target} requires duration"))
                })?;
                let value = native_boundary.coerce_arg(
                    0,
                    value,
                    &format!("{diagnostic_target} duration"),
                    heap,
                )?;
                sleep_for_millis(time_context, sleep_millis_from_runtime_value(&value)?).await?;
                native_boundary.coerce_return(
                    &RuntimeValue::Null,
                    &format!("{diagnostic_target} response"),
                    heap,
                )
            }
            _ => Err(unsupported_native_target(binding_key)),
        }
    }
}

pub(super) fn sleep_millis_from_runtime_value(value: &RuntimeValue) -> Result<u64> {
    let RuntimeValue::Number(value) = value else {
        return Err(RuntimeError::Decode(
            "std.time.sleep duration must be an integer millisecond payload".to_string(),
        ));
    };
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(RuntimeError::Decode(
            "std.time.sleep duration must be an integer millisecond payload".to_string(),
        ));
    }
    if value.abs() > 9_007_199_254_740_991.0 {
        return Err(RuntimeError::Decode(
            "std.time.sleep duration must be a safe integer millisecond payload".to_string(),
        ));
    }
    Ok(clamp_sleep_millis(*value))
}

pub(super) fn clamp_sleep_millis(value: f64) -> u64 {
    if value <= 0.0 {
        return 0;
    }
    if value >= TIME_SLEEP_MAX_MILLIS as f64 {
        return TIME_SLEEP_MAX_MILLIS;
    }
    value as u64
}

async fn sleep_for_millis(
    time_context: &(impl NativeTimeCapability + ?Sized),
    millis: u64,
) -> Result<()> {
    time_context.poll_execution_budget()?;
    if millis == 0 {
        return Ok(());
    }

    let sleep_until = std::time::Instant::now() + std::time::Duration::from_millis(millis);
    loop {
        time_context.poll_execution_budget()?;
        let now = std::time::Instant::now();
        if now >= sleep_until {
            return Ok(());
        }
        let remaining = sleep_until.saturating_duration_since(now);
        let tick = remaining.min(std::time::Duration::from_millis(TIME_SLEEP_POLL_MILLIS));
        tokio::time::sleep(tick).await;
    }
}
