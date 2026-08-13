use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, BytecodeRequestExecution,
    BytecodeRequestExecutionInput, RequestExecutionOwnerInventory,
    RequestExecutionOwnerInventorySnapshot, RequestResult,
};

pub(super) enum DrivenBytecodeRequestOwnerInventory {
    NotStarted(RequestExecutionOwnerInventorySnapshot),
    Started(RequestExecutionOwnerInventorySnapshot),
}

impl DrivenBytecodeRequestOwnerInventory {
    pub(super) fn into_snapshot(self) -> RequestExecutionOwnerInventorySnapshot {
        match self {
            Self::NotStarted(snapshot) | Self::Started(snapshot) => snapshot,
        }
    }
}

#[must_use = "the result and frozen owner inventory must reach supervisor completion"]
pub(super) struct DrivenBytecodeRequest<E = BytecodeRequestExecution> {
    pub(super) result: RequestResult<BoundaryResponse>,
    pub(super) execution: Option<E>,
    pub(super) owner_inventory: DrivenBytecodeRequestOwnerInventory,
}

pub(super) fn drive_bytecode_request(
    input: BytecodeRequestExecutionInput,
) -> DrivenBytecodeRequest {
    let (owner_registrations, owner_inventory_freeze) =
        RequestExecutionOwnerInventory::open().into_parts();
    drive_bytecode_request_with(
        move || request_runner::start_runtime_bytecode_request(input, owner_registrations),
        BytecodeRequestExecution::run,
        move || owner_inventory_freeze.freeze(),
    )
}

fn drive_bytecode_request_with<E>(
    start: impl FnOnce() -> RequestResult<E>,
    run: impl FnOnce(&mut E) -> RequestResult<BoundaryResponse>,
    freeze: impl FnOnce() -> RequestExecutionOwnerInventorySnapshot,
) -> DrivenBytecodeRequest<E> {
    let mut execution = match start() {
        Ok(execution) => execution,
        Err(error) => {
            return DrivenBytecodeRequest {
                result: Err(error),
                execution: None,
                owner_inventory: DrivenBytecodeRequestOwnerInventory::NotStarted(freeze()),
            }
        }
    };
    let result = run(&mut execution);
    DrivenBytecodeRequest {
        result,
        execution: Some(execution),
        owner_inventory: DrivenBytecodeRequestOwnerInventory::Started(freeze()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        any::Any,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use skiff_runtime_request::{RequestError, ResponseEnd, ResponseEvent};

    use super::*;

    struct FakeExecution {
        _owner: Box<dyn Any + Send>,
    }

    #[test]
    fn start_error_freezes_the_actual_not_started_inventory_without_running() {
        let run_count = Arc::new(AtomicUsize::new(0));
        let (registrations, freeze) = RequestExecutionOwnerInventory::open().into_parts();
        let registration_after_freeze = registrations.pending();
        let DrivenBytecodeRequest {
            result,
            execution,
            owner_inventory,
        } = drive_bytecode_request_with(
            move || {
                let released_owner = registrations
                    .pending()
                    .prepare()
                    .unwrap()
                    .install(|owner| owner)
                    .unwrap();
                drop(released_owner);
                Err::<FakeExecution, _>(RequestError::Decode("fake start failure".to_string()))
            },
            {
                let run_count = Arc::clone(&run_count);
                move |_| {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    Ok(BoundaryResponse::payload(Vec::new()))
                }
            },
            move || freeze.freeze(),
        );

        assert!(
            matches!(result, Err(RequestError::Decode(message)) if message == "fake start failure")
        );
        assert!(execution.is_none());
        assert_eq!(run_count.load(Ordering::SeqCst), 0);
        let DrivenBytecodeRequestOwnerInventory::NotStarted(snapshot) = owner_inventory else {
            panic!("a start failure must carry NotStarted");
        };
        assert_eq!(snapshot.pending().current(), 0);
        assert!(snapshot.pending().ever_created());
        assert_eq!(snapshot.resource().current(), 0);
        assert!(!snapshot.resource().ever_created());
        assert_eq!(snapshot.child().current(), 0);
        assert!(!snapshot.child().ever_created());
        assert!(registration_after_freeze.prepare().is_err());
    }

    #[test]
    fn run_success_executes_once_and_freezes_the_actual_started_inventory() {
        let run_count = Arc::new(AtomicUsize::new(0));
        let (registrations, freeze) = RequestExecutionOwnerInventory::open().into_parts();
        let registration_after_freeze = registrations.child();
        let DrivenBytecodeRequest {
            result,
            execution,
            owner_inventory,
        } = drive_bytecode_request_with(
            move || {
                let owner = registrations
                    .pending()
                    .prepare()
                    .unwrap()
                    .install(|owner| Box::new(owner) as Box<dyn Any + Send>)
                    .unwrap();
                Ok(FakeExecution { _owner: owner })
            },
            {
                let run_count = Arc::clone(&run_count);
                move |_| {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    Ok(BoundaryResponse::payload(b"done".to_vec()))
                }
            },
            move || freeze.freeze(),
        );

        assert_eq!(run_count.load(Ordering::SeqCst), 1);
        assert!(matches!(
            result,
            Ok(BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload))))
                if payload == b"done"
        ));
        let DrivenBytecodeRequestOwnerInventory::Started(snapshot) = owner_inventory else {
            panic!("a started execution must carry Started");
        };
        assert_eq!(snapshot.pending().current(), 1);
        assert!(snapshot.pending().ever_created());
        drop(execution);
        assert_eq!(snapshot.pending().current(), 1);
        assert!(registration_after_freeze.prepare().is_err());
    }

    #[test]
    fn run_error_executes_once_and_still_freezes_started() {
        let run_count = Arc::new(AtomicUsize::new(0));
        let (registrations, freeze) = RequestExecutionOwnerInventory::open().into_parts();
        let registration_after_freeze = registrations.resource();
        let DrivenBytecodeRequest {
            result,
            execution,
            owner_inventory,
        } = drive_bytecode_request_with(
            move || {
                let owner = registrations
                    .resource()
                    .prepare()
                    .unwrap()
                    .install(|owner| Box::new(owner) as Box<dyn Any + Send>)
                    .unwrap();
                Ok(FakeExecution { _owner: owner })
            },
            {
                let run_count = Arc::clone(&run_count);
                move |_| {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    Err(RequestError::Decode("fake run failure".to_string()))
                }
            },
            move || freeze.freeze(),
        );

        assert!(
            matches!(result, Err(RequestError::Decode(message)) if message == "fake run failure")
        );
        assert!(execution.is_some());
        assert_eq!(run_count.load(Ordering::SeqCst), 1);
        let DrivenBytecodeRequestOwnerInventory::Started(snapshot) = owner_inventory else {
            panic!("a run failure must carry Started");
        };
        assert_eq!(snapshot.resource().current(), 1);
        assert!(snapshot.resource().ever_created());
        assert!(registration_after_freeze.prepare().is_err());
    }
}
