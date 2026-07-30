use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Waker},
};

use mongodb::bson::{doc, Document};

use crate::{
    prepared_runtime::{
        PreparedRuntimeTestDriver, PreparedRuntimeTestKind, PreparedRuntimeTestOutcome,
        PreparedRuntimeTestWait,
    },
    Result, ServiceDbError,
};

#[derive(Clone)]
pub(super) struct TestDriver {
    state: Arc<TestDriverState>,
}

struct TestDriverState {
    ready: AtomicBool,
    fail: AtomicBool,
    starts: AtomicUsize,
    completions: AtomicUsize,
    pending_drops: AtomicUsize,
    kinds: Mutex<Vec<PreparedRuntimeTestKind>>,
    waker: Mutex<Option<Waker>>,
    document: Document,
}

impl TestDriver {
    pub(super) fn pending() -> Self {
        Self::new(false, default_document())
    }

    pub(super) fn ready() -> Self {
        Self::new(true, default_document())
    }

    pub(super) fn ready_with_document(document: Document) -> Self {
        Self::new(true, document)
    }

    fn new(ready: bool, document: Document) -> Self {
        Self {
            state: Arc::new(TestDriverState {
                ready: AtomicBool::new(ready),
                fail: AtomicBool::new(false),
                starts: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                pending_drops: AtomicUsize::new(0),
                kinds: Mutex::new(Vec::new()),
                waker: Mutex::new(None),
                document,
            }),
        }
    }

    pub(super) fn fail(&self) {
        self.state.fail.store(true, Ordering::SeqCst);
        self.release();
    }

    pub(super) fn release(&self) {
        self.state.ready.store(true, Ordering::SeqCst);
        if let Some(waker) = self.state.waker.lock().expect("driver waker mutex").take() {
            waker.wake();
        }
    }

    pub(super) async fn wait_until_started(&self) {
        for _ in 0..100 {
            if self.starts() > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("prepared runtime test wait did not start");
    }

    pub(super) fn starts(&self) -> usize {
        self.state.starts.load(Ordering::SeqCst)
    }

    pub(super) fn completions(&self) -> usize {
        self.state.completions.load(Ordering::SeqCst)
    }

    pub(super) fn pending_drops(&self) -> usize {
        self.state.pending_drops.load(Ordering::SeqCst)
    }

    pub(super) fn kinds(&self) -> Vec<PreparedRuntimeTestKind> {
        self.state.kinds.lock().expect("driver kinds mutex").clone()
    }
}

impl PreparedRuntimeTestDriver for TestDriver {
    fn wait(&self, kind: PreparedRuntimeTestKind) -> PreparedRuntimeTestWait {
        Box::pin(TestDriverWait {
            driver: self.clone(),
            kind,
            started: false,
            done: false,
        })
    }
}

struct TestDriverWait {
    driver: TestDriver,
    kind: PreparedRuntimeTestKind,
    started: bool,
    done: bool,
}

impl Future for TestDriverWait {
    type Output = Result<PreparedRuntimeTestOutcome>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.started {
            self.started = true;
            self.driver.state.starts.fetch_add(1, Ordering::SeqCst);
            self.driver
                .state
                .kinds
                .lock()
                .expect("driver kinds mutex")
                .push(self.kind);
        }
        if !self.driver.state.ready.load(Ordering::SeqCst) {
            *self.driver.state.waker.lock().expect("driver waker mutex") =
                Some(context.waker().clone());
            return Poll::Pending;
        }
        self.done = true;
        if self.driver.state.fail.load(Ordering::SeqCst) {
            return Poll::Ready(Err(ServiceDbError::Decode(
                "prepared runtime test provider failure".to_string(),
            )));
        }
        self.driver.state.completions.fetch_add(1, Ordering::SeqCst);
        let document = self.driver.state.document.clone();
        let outcome = match self.kind {
            PreparedRuntimeTestKind::FindMany => PreparedRuntimeTestOutcome::Many(vec![document]),
            PreparedRuntimeTestKind::Create => PreparedRuntimeTestOutcome::Create,
            PreparedRuntimeTestKind::FindOneByKey
            | PreparedRuntimeTestKind::FindOneByQuery
            | PreparedRuntimeTestKind::Update
            | PreparedRuntimeTestKind::Replace => {
                PreparedRuntimeTestOutcome::Optional(Some(document))
            }
        };
        Poll::Ready(Ok(outcome))
    }
}

impl Drop for TestDriverWait {
    fn drop(&mut self) {
        if self.started && !self.done {
            self.driver
                .state
                .pending_drops
                .fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn default_document() -> Document {
    doc! {
        "_id": "item-1",
        "title": "from-provider",
    }
}
