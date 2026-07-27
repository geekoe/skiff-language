use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

use skiff_runtime_boundary::request_heap::RequestHeap;

use super::{continuation_error, ActorExecutionFrame};
use crate::error::RuntimeError;

pub(crate) struct ActorConcurrentContinuationBridge {
    parent: ActorExecutionFrame,
    lanes: Vec<ActorExecutionFrame>,
    claims: Vec<AtomicBool>,
    gate: Arc<ActorConcurrentContinuationGate>,
}

impl ActorConcurrentContinuationBridge {
    pub(super) fn begin(
        parent: &ActorExecutionFrame,
        heap: &RequestHeap,
        lane_count: usize,
    ) -> Result<Self, RuntimeError> {
        if lane_count == 0 {
            return Err(continuation_error(
                "Actor concurrent continuation requires at least one lane",
            ));
        }
        if parent.suspension.child.is_some() {
            return Err(continuation_error(
                "Actor child continuation cannot create a nested outer bridge",
            ));
        }

        let gate = Arc::new(ActorConcurrentContinuationGate::new(lane_count));
        {
            let mut current = parent
                .suspension
                .outer_gate
                .lock()
                .expect("actor concurrent continuation gate lock poisoned");
            if current
                .as_ref()
                .is_some_and(|active| active.remaining_children.load(Ordering::Acquire) != 0)
            {
                return Err(continuation_error(
                    "Actor continuation already has active concurrent children",
                ));
            }
            *current = Some(Arc::clone(&gate));
        }
        if let Err(error) = parent.suspend(heap) {
            let mut current = parent
                .suspension
                .outer_gate
                .lock()
                .expect("actor concurrent continuation gate lock poisoned");
            if current
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &gate))
            {
                *current = None;
            }
            return Err(error);
        }

        let lanes = (0..lane_count)
            .map(|_| {
                let child = Arc::new(ActorChildContinuationState::new(Arc::clone(&gate)));
                ActorExecutionFrame::suspended_child(parent, child)
            })
            .collect();
        Ok(Self {
            parent: parent.clone(),
            lanes,
            claims: (0..lane_count).map(|_| AtomicBool::new(false)).collect(),
            gate,
        })
    }

    pub(crate) fn lane(
        &self,
        index: usize,
    ) -> Result<ActorConcurrentContinuationLane, RuntimeError> {
        let frame = self.lanes.get(index).ok_or_else(|| {
            continuation_error(format!(
                "Actor concurrent continuation lane {index} is out of range"
            ))
        })?;
        let claimed = &self.claims[index];
        if claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(continuation_error(format!(
                "Actor concurrent continuation lane {index} was already claimed"
            )));
        }
        Ok(ActorConcurrentContinuationLane {
            frame: frame.clone(),
        })
    }

    pub(crate) async fn resume_parent(
        &self,
        heap: &mut RequestHeap,
        execution: &crate::capabilities::ExecutionControl<'_>,
    ) -> Result<(), RuntimeError> {
        self.gate.ensure_released()?;
        self.parent.resume(heap, execution).await
    }
}

impl Drop for ActorConcurrentContinuationBridge {
    fn drop(&mut self) {
        for lane in &self.lanes {
            lane.abandon_child();
        }
    }
}

pub(crate) struct ActorConcurrentContinuationLane {
    frame: ActorExecutionFrame,
}

impl ActorConcurrentContinuationLane {
    pub(crate) fn frame(&self) -> &ActorExecutionFrame {
        &self.frame
    }

    pub(crate) async fn resume(
        &self,
        heap: &mut RequestHeap,
        execution: &crate::capabilities::ExecutionControl<'_>,
    ) -> Result<(), RuntimeError> {
        self.frame.resume(heap, execution).await
    }

    pub(crate) fn complete(self, heap: RequestHeap) -> Result<(), RuntimeError> {
        self.frame.finish(heap)
    }

    pub(crate) fn abandon(self) {}
}

impl Drop for ActorConcurrentContinuationLane {
    fn drop(&mut self) {
        self.frame.abandon_child();
    }
}

pub(super) struct ActorConcurrentContinuationGate {
    remaining_children: AtomicUsize,
    active_segments: AtomicUsize,
}

impl ActorConcurrentContinuationGate {
    fn new(lane_count: usize) -> Self {
        Self {
            remaining_children: AtomicUsize::new(lane_count),
            active_segments: AtomicUsize::new(0),
        }
    }

    pub(super) fn ensure_released(&self) -> Result<(), RuntimeError> {
        let remaining = self.remaining_children.load(Ordering::Acquire);
        if remaining == 0 {
            return Ok(());
        }
        let active = self.active_segments.load(Ordering::Acquire);
        Err(continuation_error(format!(
            "Actor outer continuation cannot resume while {remaining} concurrent child lane(s) remain active ({active} synchronous segment(s) held)"
        )))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActorChildContinuationPhase {
    Suspended,
    Acquiring,
    HoldingSegment,
    Finished,
}

pub(super) struct ActorChildContinuationState {
    gate: Arc<ActorConcurrentContinuationGate>,
    phase: Mutex<ActorChildContinuationPhase>,
}

impl ActorChildContinuationState {
    fn new(gate: Arc<ActorConcurrentContinuationGate>) -> Self {
        Self {
            gate,
            phase: Mutex::new(ActorChildContinuationPhase::Suspended),
        }
    }

    pub(super) fn begin_resume(&self) -> Result<ActorChildResumePermit<'_>, RuntimeError> {
        let mut phase = self
            .phase
            .lock()
            .expect("actor child continuation state lock poisoned");
        match *phase {
            ActorChildContinuationPhase::Suspended => {
                *phase = ActorChildContinuationPhase::Acquiring;
                Ok(ActorChildResumePermit {
                    child: self,
                    acquired: false,
                })
            }
            ActorChildContinuationPhase::Acquiring => Err(continuation_error(
                "Actor child continuation is already acquiring a synchronous segment",
            )),
            ActorChildContinuationPhase::HoldingSegment => Err(continuation_error(
                "Actor child continuation already owns a synchronous segment",
            )),
            ActorChildContinuationPhase::Finished => Err(continuation_error(
                "Actor child continuation cannot resume after completion or abandonment",
            )),
        }
    }

    pub(super) fn segment_released(&self) {
        let mut phase = self
            .phase
            .lock()
            .expect("actor child continuation state lock poisoned");
        if *phase == ActorChildContinuationPhase::HoldingSegment {
            *phase = ActorChildContinuationPhase::Suspended;
            self.gate.active_segments.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub(super) fn finish(&self) {
        let mut phase = self
            .phase
            .lock()
            .expect("actor child continuation state lock poisoned");
        if *phase == ActorChildContinuationPhase::Finished {
            return;
        }
        if *phase == ActorChildContinuationPhase::HoldingSegment {
            self.gate.active_segments.fetch_sub(1, Ordering::AcqRel);
        }
        *phase = ActorChildContinuationPhase::Finished;
        self.gate.remaining_children.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct ActorChildResumePermit<'a> {
    child: &'a ActorChildContinuationState,
    acquired: bool,
}

impl ActorChildResumePermit<'_> {
    pub(super) fn segment_acquired(&mut self) -> Result<(), RuntimeError> {
        let mut phase = self
            .child
            .phase
            .lock()
            .expect("actor child continuation state lock poisoned");
        match *phase {
            ActorChildContinuationPhase::Acquiring => {
                *phase = ActorChildContinuationPhase::HoldingSegment;
                self.child
                    .gate
                    .active_segments
                    .fetch_add(1, Ordering::AcqRel);
                self.acquired = true;
                Ok(())
            }
            ActorChildContinuationPhase::Finished => Err(continuation_error(
                "Actor child continuation was abandoned while acquiring its synchronous segment",
            )),
            ActorChildContinuationPhase::Suspended
            | ActorChildContinuationPhase::HoldingSegment => Err(continuation_error(
                "Actor child continuation acquire state changed unexpectedly",
            )),
        }
    }
}

impl Drop for ActorChildResumePermit<'_> {
    fn drop(&mut self) {
        if self.acquired {
            return;
        }
        let mut phase = self
            .child
            .phase
            .lock()
            .expect("actor child continuation state lock poisoned");
        if *phase == ActorChildContinuationPhase::Acquiring {
            *phase = ActorChildContinuationPhase::Suspended;
        }
    }
}
