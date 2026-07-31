use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use super::*;
use tokio::sync::oneshot;

mod evaluator_actual_pending;
mod evaluator_concurrent;

fn executable_addr() -> ExecutableAddr {
    ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    }
}

fn write_count(fixture: &Fixture, frame: &ActorExecutionFrame, heap: &mut RequestHeap, value: f64) {
    let program = fixture.interpreter.program_projection().unwrap();
    frame
        .write_field(
            "count",
            &integer(),
            program.type_view(),
            &executable_addr(),
            &RuntimeValue::Number(value),
            heap,
        )
        .unwrap();
}

struct TrackedOneshot {
    receiver: oneshot::Receiver<&'static str>,
    first_poll: Option<oneshot::Sender<()>>,
    active_count: Arc<AtomicUsize>,
    active: bool,
}

impl TrackedOneshot {
    fn new(
        receiver: oneshot::Receiver<&'static str>,
        first_poll: oneshot::Sender<()>,
        active_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            receiver,
            first_poll: Some(first_poll),
            active_count,
            active: false,
        }
    }

    fn leave_active(&mut self) {
        if self.active {
            self.active_count.fetch_sub(1, Ordering::SeqCst);
            self.active = false;
        }
    }
}

impl Future for TrackedOneshot {
    type Output = Result<&'static str, oneshot::error::RecvError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.active {
            this.active_count.fetch_add(1, Ordering::SeqCst);
            this.active = true;
            if let Some(first_poll) = this.first_poll.take() {
                let _ = first_poll.send(());
            }
        }
        match Pin::new(&mut this.receiver).poll(context) {
            Poll::Ready(output) => {
                this.leave_active();
                Poll::Ready(output)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for TrackedOneshot {
    fn drop(&mut self) {
        self.leave_active();
    }
}

#[tokio::test]
async fn actor_concurrent_continuation_parent_suspends_once_and_children_are_independent() {
    let fixture = fixture(integer(), true);
    let (parent, mut heap) = execution_frame(&fixture).await;
    write_count(&fixture, &parent, &mut heap, 3.0);

    let bridge = parent.begin_concurrent(&heap, 2).unwrap();
    let first = bridge.lane(0).unwrap();
    let second = bridge.lane(1).unwrap();

    assert!(!parent.has_execution_lease());
    assert!(!first.frame().has_execution_lease());
    assert!(!second.frame().has_execution_lease());
    assert!(!first.frame().shares_execution_slot(second.frame()));

    let execution = context(&fixture.interpreter).execution();
    let mut child_heap = heap.clone();
    first.resume(&mut child_heap, &execution).await.unwrap();
    assert_eq!(
        first.frame().read_field("count").unwrap(),
        RuntimeValue::Number(3.0),
        "the parent synchronous segment must be committed before any child acquires"
    );
    first.complete(child_heap).unwrap();
    second.abandon();
    bridge.resume_parent(&mut heap, &execution).await.unwrap();
    parent.finish(heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_nested_bridge_composes_both_gates_and_commits() {
    let fixture = fixture(integer(), true);
    let (parent, mut parent_heap) = execution_frame(&fixture).await;
    let execution = context(&fixture.interpreter).execution();
    let outer_bridge = parent.begin_concurrent(&parent_heap, 1).unwrap();
    let outer_lane = outer_bridge.lane(0).unwrap();
    let mut outer_lane_heap = parent_heap.clone();
    outer_lane
        .resume(&mut outer_lane_heap, &execution)
        .await
        .unwrap();
    write_count(&fixture, outer_lane.frame(), &mut outer_lane_heap, 2.0);

    let nested_bridge = outer_lane
        .frame()
        .begin_concurrent(&outer_lane_heap, 2)
        .unwrap();
    assert!(!outer_lane.frame().has_execution_lease());
    let nested_first = nested_bridge.lane(0).unwrap();
    let nested_second = nested_bridge.lane(1).unwrap();

    let mut nested_first_heap = outer_lane_heap.clone();
    nested_first
        .resume(&mut nested_first_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        nested_first.frame().read_field("count").unwrap(),
        RuntimeValue::Number(2.0)
    );
    write_count(&fixture, nested_first.frame(), &mut nested_first_heap, 3.0);
    nested_first.complete(nested_first_heap).unwrap();

    let mut nested_second_heap = outer_lane_heap.clone();
    nested_second
        .resume(&mut nested_second_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        nested_second.frame().read_field("count").unwrap(),
        RuntimeValue::Number(3.0)
    );
    write_count(
        &fixture,
        nested_second.frame(),
        &mut nested_second_heap,
        4.0,
    );
    nested_second.complete(nested_second_heap).unwrap();

    let error = outer_bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("(0 synchronous segment(s) held)"));

    nested_bridge
        .resume_parent(&mut outer_lane_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        outer_lane.frame().read_field("count").unwrap(),
        RuntimeValue::Number(4.0)
    );
    let error = outer_bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("(1 synchronous segment(s) held)"));
    write_count(&fixture, outer_lane.frame(), &mut outer_lane_heap, 5.0);
    outer_lane.complete(outer_lane_heap).unwrap();

    outer_bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        parent.read_field("count").unwrap(),
        RuntimeValue::Number(5.0)
    );
    parent.finish(parent_heap).unwrap();
    assert_eq!(
        execute(&fixture, &fixture.method, b"[6]").await.unwrap(),
        b"6",
        "nested completion must leave no scheduler guard behind"
    );
}

#[tokio::test]
async fn actor_concurrent_continuation_resume_with_installed_lease_fails_on_first_poll() {
    let fixture = fixture(integer(), true);
    let (frame, heap) = execution_frame(&fixture).await;
    let execution = context(&fixture.interpreter).execution();
    let mut resume_heap = heap.clone();

    let first_poll = {
        let resume = frame.resume(&mut resume_heap, &execution);
        tokio::pin!(resume);
        std::future::poll_fn(|context| Poll::Ready(resume.as_mut().poll(context))).await
    };
    let Poll::Ready(Err(error)) = first_poll else {
        panic!("resume with an installed lease must fail before scheduler acquisition");
    };
    assert!(matches!(
        error,
        RuntimeError::InvalidArtifact(message)
            if message
                == "Actor continuation attempted to resume while an execution token is already installed"
    ));
    assert!(frame.has_execution_lease());
    frame.finish(heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_serializes_segments_but_overlaps_pending_futures() {
    let fixture = fixture(integer(), true);
    let (parent, parent_heap) = execution_frame(&fixture).await;
    let bridge = parent.begin_concurrent(&parent_heap, 2).unwrap();
    let first = bridge.lane(0).unwrap();
    let second = bridge.lane(1).unwrap();
    let mut first_heap = parent_heap.clone();
    let mut second_heap = parent_heap.clone();
    let first_execution = context(&fixture.interpreter).execution();
    let second_execution = context(&fixture.interpreter).execution();

    first
        .resume(&mut first_heap, &first_execution)
        .await
        .unwrap();
    let active_futures = Arc::new(AtomicUsize::new(0));
    let (second_acquired_tx, mut second_acquired_rx) = oneshot::channel();
    let (second_polled_tx, second_polled_rx) = oneshot::channel();
    let (second_ready_tx, second_ready_rx) = oneshot::channel();
    let second_active = Arc::clone(&active_futures);
    let second_task = tokio::spawn(async move {
        second
            .resume(&mut second_heap, &second_execution)
            .await
            .unwrap();
        second_acquired_tx.send(()).unwrap();
        let output = second
            .frame()
            .await_if_pending(
                &mut second_heap,
                &second_execution,
                TrackedOneshot::new(second_ready_rx, second_polled_tx, second_active),
            )
            .await
            .unwrap()
            .unwrap();
        second.complete(second_heap).unwrap();
        output
    });

    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut second_acquired_rx)
            .await
            .is_err(),
        "a second child must not enter while the first owns a synchronous segment"
    );

    let (first_polled_tx, first_polled_rx) = oneshot::channel();
    let (first_ready_tx, first_ready_rx) = oneshot::channel();
    let first_active = Arc::clone(&active_futures);
    let first_task = tokio::spawn(async move {
        let output = first
            .frame()
            .await_if_pending(
                &mut first_heap,
                &first_execution,
                TrackedOneshot::new(first_ready_rx, first_polled_tx, first_active),
            )
            .await
            .unwrap()
            .unwrap();
        first.complete(first_heap).unwrap();
        output
    });

    first_polled_rx.await.unwrap();
    second_acquired_rx.await.unwrap();
    second_polled_rx.await.unwrap();
    assert_eq!(
        active_futures.load(Ordering::SeqCst),
        2,
        "both external operations must remain pending after their Actor segments release"
    );

    first_ready_tx.send("first").unwrap();
    second_ready_tx.send("second").unwrap();
    assert_eq!(first_task.await.unwrap(), "first");
    assert_eq!(second_task.await.unwrap(), "second");

    let mut parent_heap = parent_heap;
    let execution = context(&fixture.interpreter).execution();
    bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap();
    parent.finish(parent_heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_ready_future_keeps_the_current_segment() {
    let fixture = fixture(integer(), true);
    let (parent, parent_heap) = execution_frame(&fixture).await;
    let bridge = parent.begin_concurrent(&parent_heap, 2).unwrap();
    let first = bridge.lane(0).unwrap();
    let second = bridge.lane(1).unwrap();
    let mut first_heap = parent_heap.clone();
    let mut second_heap = parent_heap.clone();
    let first_execution = context(&fixture.interpreter).execution();
    let second_execution = context(&fixture.interpreter).execution();

    first
        .resume(&mut first_heap, &first_execution)
        .await
        .unwrap();
    assert_eq!(
        first
            .frame()
            .await_if_pending(&mut first_heap, &first_execution, async { "buffered" })
            .await
            .unwrap(),
        "buffered"
    );
    assert!(first.frame().has_execution_lease());

    let (acquired_tx, mut acquired_rx) = oneshot::channel();
    let second_task = tokio::spawn(async move {
        second
            .resume(&mut second_heap, &second_execution)
            .await
            .unwrap();
        acquired_tx.send(()).unwrap();
        second.complete(second_heap).unwrap();
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut acquired_rx)
            .await
            .is_err(),
        "a ready future must not commit or release the first child segment"
    );

    first.complete(first_heap).unwrap();
    acquired_rx.await.unwrap();
    second_task.await.unwrap();

    let mut parent_heap = parent_heap;
    let execution = context(&fixture.interpreter).execution();
    bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap();
    parent.finish(parent_heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_commits_children_in_order_and_preserves_outer_heap() {
    let fixture = fixture(integer(), true);
    let (parent, mut parent_heap) = execution_frame(&fixture).await;
    let local_handle = parent_heap
        .alloc_array(vec![RuntimeValue::String("outer-local".to_string())])
        .unwrap();
    let bridge = parent.begin_concurrent(&parent_heap, 2).unwrap();
    let first = bridge.lane(0).unwrap();
    let second = bridge.lane(1).unwrap();
    let execution = context(&fixture.interpreter).execution();

    let mut first_heap = parent_heap.clone();
    first.resume(&mut first_heap, &execution).await.unwrap();
    write_count(&fixture, first.frame(), &mut first_heap, 5.0);
    first.complete(first_heap).unwrap();

    let mut second_heap = parent_heap.clone();
    second.resume(&mut second_heap, &execution).await.unwrap();
    assert_eq!(
        second.frame().read_field("count").unwrap(),
        RuntimeValue::Number(5.0)
    );
    write_count(&fixture, second.frame(), &mut second_heap, 7.0);
    second.complete(second_heap).unwrap();

    bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        parent.read_field("count").unwrap(),
        RuntimeValue::Number(7.0)
    );
    assert!(matches!(
        parent_heap.get(local_handle).unwrap(),
        skiff_runtime_model::runtime_value::HeapNode::Array(items)
            if items == &[RuntimeValue::String("outer-local".to_string())]
    ));
    parent.finish(parent_heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_error_and_drop_release_without_double_commit() {
    let fixture = fixture(integer(), true);
    let (parent, mut parent_heap) = execution_frame(&fixture).await;
    let bridge = parent.begin_concurrent(&parent_heap, 3).unwrap();
    let error_lane = bridge.lane(0).unwrap();
    let held_drop = bridge.lane(1).unwrap();
    let suspended_drop = bridge.lane(2).unwrap();
    let execution = context(&fixture.interpreter).execution();

    let mut error_heap = parent_heap.clone();
    error_lane
        .resume(&mut error_heap, &execution)
        .await
        .unwrap();
    write_count(&fixture, error_lane.frame(), &mut error_heap, 9.0);
    let error = bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("child lane"));
    error_lane.abandon();

    let mut held_heap = parent_heap.clone();
    held_drop.resume(&mut held_heap, &execution).await.unwrap();
    assert_eq!(
        held_drop.frame().read_field("count").unwrap(),
        RuntimeValue::Number(1.0),
        "abandoning an errored lane must roll its held segment back"
    );
    write_count(&fixture, held_drop.frame(), &mut held_heap, 8.0);
    drop(held_drop);

    let mut suspended_heap = parent_heap.clone();
    suspended_drop
        .resume(&mut suspended_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        suspended_drop.frame().read_field("count").unwrap(),
        RuntimeValue::Number(1.0),
        "dropping a held lease must roll its field mutation back"
    );
    write_count(&fixture, suspended_drop.frame(), &mut suspended_heap, 5.0);
    suspended_drop.frame().suspend(&suspended_heap).unwrap();
    drop(suspended_drop);

    bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        parent.read_field("count").unwrap(),
        RuntimeValue::Number(5.0),
        "dropping an already suspended child must not commit a second time"
    );
    parent.finish(parent_heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_cancel_and_budget_fail_without_reinstalling_leases() {
    let fixture = fixture(integer(), true);
    let (parent, mut parent_heap) = execution_frame(&fixture).await;
    let bridge = parent.begin_concurrent(&parent_heap, 2).unwrap();
    let cancelled = bridge.lane(0).unwrap();
    let expired = bridge.lane(1).unwrap();

    let cancelled_execution = context(&fixture.interpreter).execution();
    let mut cancelled_heap = parent_heap.clone();
    cancelled
        .resume(&mut cancelled_heap, &cancelled_execution)
        .await
        .unwrap();
    cancelled.frame().suspend(&cancelled_heap).unwrap();
    cancelled_execution
        .cancel_flag()
        .store(true, Ordering::Release);
    let error = cancelled
        .resume(&mut cancelled_heap, &cancelled_execution)
        .await
        .unwrap_err();
    assert!(error.is_cancelled());
    assert!(!cancelled.frame().has_execution_lease());
    let retry_execution = context(&fixture.interpreter).execution();
    cancelled
        .resume(&mut cancelled_heap, &retry_execution)
        .await
        .unwrap();
    drop(cancelled);

    let mut expired_heap = parent_heap.clone();
    let running_execution = context(&fixture.interpreter).execution();
    expired
        .resume(&mut expired_heap, &running_execution)
        .await
        .unwrap();
    expired.frame().suspend(&expired_heap).unwrap();
    let expired_execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - Duration::from_secs(1),
    ));
    let error = expired
        .resume(&mut expired_heap, &expired_execution)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::ExecutionBudgetExceeded {
            reason: crate::error::BudgetReason::DeadlineExceeded,
            ..
        }
    ));
    assert!(!expired.frame().has_execution_lease());
    drop(expired);

    let execution = context(&fixture.interpreter).execution();
    bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap();
    parent.finish(parent_heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_winner_abandons_all_children_before_outer_resume() {
    let fixture = fixture(integer(), true);
    let (parent, mut parent_heap) = execution_frame(&fixture).await;
    let bridge = parent.begin_concurrent(&parent_heap, 3).unwrap();

    bridge.lane(0).unwrap().abandon();
    bridge.lane(1).unwrap().abandon();
    bridge.lane(2).unwrap().abandon();

    let execution = context(&fixture.interpreter).execution();
    bridge
        .resume_parent(&mut parent_heap, &execution)
        .await
        .unwrap();
    assert_eq!(
        parent.read_field("count").unwrap(),
        RuntimeValue::Number(1.0)
    );
    parent.finish(parent_heap).unwrap();
}

#[tokio::test]
async fn actor_concurrent_continuation_rejects_replacement_and_stale_epoch_without_a_lease() {
    let replaced = fixture(integer(), true);
    let (replaced_parent, replaced_heap) = execution_frame(&replaced).await;
    let replaced_bridge = replaced_parent.begin_concurrent(&replaced_heap, 1).unwrap();
    let replaced_lane = replaced_bridge.lane(0).unwrap();
    let mut replaced_child_heap = replaced_heap.clone();
    let execution = context(&replaced.interpreter).execution();
    replaced_lane
        .resume(&mut replaced_child_heap, &execution)
        .await
        .unwrap();
    replaced_lane.frame().suspend(&replaced_child_heap).unwrap();
    assert!(replaced.store.begin_upgrade_exact(&replaced.handle));
    let error = replaced_lane
        .resume(&mut replaced_child_heap, &execution)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::ActorInstance(ActorInstanceStoreError::InstanceReplaced)
    ));
    assert!(!replaced_lane.frame().has_execution_lease());
    drop(replaced_lane);
    drop(replaced_bridge);

    let stale = fixture(integer(), true);
    let (stale_parent, stale_heap) = execution_frame(&stale).await;
    let stale_bridge = stale_parent.begin_concurrent(&stale_heap, 1).unwrap();
    let stale_lane = stale_bridge.lane(0).unwrap();
    let mut stale_child_heap = stale_heap.clone();
    let execution = context(&stale.interpreter).execution();
    stale_lane
        .resume(&mut stale_child_heap, &execution)
        .await
        .unwrap();
    stale_lane.frame().suspend(&stale_child_heap).unwrap();
    let mut newer_fence = stale.handle.fence().clone();
    newer_fence.incarnation.epoch = 2;
    let program = stale.interpreter.program_projection().unwrap();
    stale
        .store
        .activate(ActorActivationRequest {
            fence: newer_fence,
            bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
            bootstrap_payload: br#"[]"#,
            program: program.type_view(),
        })
        .unwrap();
    let error = stale_lane
        .resume(&mut stale_child_heap, &execution)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::ActorInstance(ActorInstanceStoreError::StaleEpoch { .. })
    ));
    assert!(!stale_lane.frame().has_execution_lease());
}
