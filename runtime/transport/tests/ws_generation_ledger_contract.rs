//! TEST-ONLY reference model for `RuntimeGenerationPinLedger`
//! (C-ws §3, `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-ws-contract.md`;
//! M4: the per-connection generation pin is keyed by buildId, not the
//! retired assembly generation).
//!
//! Reference model of the retired ws generation lifecycle semantics
//! (expect/acquire/release pending/cached acquire/session attachment/release
//! timeout/runtime disconnect/flush; the on-wire lifecycle protocol was
//! removed in W8b). Not production code.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct PinTuple {
        router_session_id: String,
        service_id: String,
        build_id: String,
        websocket_entry_id: String,
        connection_id: String,
    }

    impl PinTuple {
        fn new(connection_id: &str, router_session_id: &str) -> Self {
            Self {
                router_session_id: router_session_id.to_string(),
                service_id: "example.com/chat".to_string(),
                build_id: "skiff-deployment-artifact-v4:sha256:7".to_string(),
                websocket_entry_id: "entry-1".to_string(),
                connection_id: connection_id.to_string(),
            }
        }

        fn with_build_id(mut self, build_id: &str) -> Self {
            self.build_id = build_id.to_string();
            self
        }

        fn with_connection(mut self, connection_id: &str) -> Self {
            self.connection_id = connection_id.to_string();
            self
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RejectCode {
        NotAcquired,
        RequestConflict,
        SenderMismatch,
        TupleMismatch,
    }

    #[derive(Debug, Clone)]
    enum AcquireResponse {
        Ack,
        Reject(RejectCode),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ReleaseState {
        Pending,
        Resolved,
    }

    #[derive(Debug, Clone)]
    struct PendingRelease {
        connection_id: String,
        request_id: String,
        runtime: String,
        state: ReleaseState,
    }

    #[derive(Debug, Clone)]
    struct Acquired {
        tuple: PinTuple,
        runtime: String,
    }

    #[derive(Debug, Default)]
    struct LedgerRef {
        expected: HashMap<String, PinTuple>,
        acquired: HashMap<String, Acquired>,
        pending_by_connection: HashMap<String, PendingRelease>,
        pending_by_request: HashMap<String, String>,
        cached: HashMap<String, (String, PinTuple, String)>,
        session_by_runtime: HashMap<String, String>,
        runtime_by_session: HashMap<String, String>,
        ack_counts: HashMap<String, u64>,
        failures: Vec<String>,
        disconnect_handlers_called: Vec<String>,
        next_request_id: u64,
        fail_stop: bool,
    }

    impl LedgerRef {
        fn expect_connection(&mut self, tuple: PinTuple) {
            if self.expected.contains_key(&tuple.connection_id) {
                self.fail_stop = true;
                return;
            }
            self.expected.insert(tuple.connection_id.clone(), tuple);
        }

        fn acquire(
            &mut self,
            runtime: &str,
            request_id: &str,
            tuple: &PinTuple,
        ) -> AcquireResponse {
            if let Some((cached_connection, cached_tuple, cached_runtime)) =
                self.cached.get(request_id)
            {
                if cached_runtime == runtime
                    && cached_connection == &tuple.connection_id
                    && cached_tuple == tuple
                {
                    return AcquireResponse::Ack;
                }
                return AcquireResponse::Reject(RejectCode::RequestConflict);
            }

            let response = self.acquire_response(runtime, tuple);
            if matches!(response, AcquireResponse::Ack) {
                self.cached.insert(
                    request_id.to_string(),
                    (
                        tuple.connection_id.clone(),
                        tuple.clone(),
                        runtime.to_string(),
                    ),
                );
                self.acquired.insert(
                    tuple.connection_id.clone(),
                    Acquired {
                        tuple: tuple.clone(),
                        runtime: runtime.to_string(),
                    },
                );
                self.session_by_runtime
                    .insert(runtime.to_string(), tuple.router_session_id.clone());
                self.runtime_by_session
                    .insert(tuple.router_session_id.clone(), runtime.to_string());
            }
            response
        }

        fn acquire_response(&self, runtime: &str, tuple: &PinTuple) -> AcquireResponse {
            let session_runtime = self.runtime_by_session.get(&tuple.router_session_id);
            if let Some(existing) = session_runtime {
                if existing != runtime {
                    return AcquireResponse::Reject(RejectCode::SenderMismatch);
                }
            }
            if let Some(existing) = self.session_by_runtime.get(runtime) {
                if existing != &tuple.router_session_id {
                    return AcquireResponse::Reject(RejectCode::SenderMismatch);
                }
            }
            let Some(expected) = self.expected.get(&tuple.connection_id) else {
                return AcquireResponse::Reject(RejectCode::NotAcquired);
            };
            if expected != tuple {
                return AcquireResponse::Reject(RejectCode::TupleMismatch);
            }
            if let Some(existing) = self.acquired.get(&tuple.connection_id) {
                if existing.runtime != runtime || existing.tuple != *tuple {
                    return AcquireResponse::Reject(RejectCode::TupleMismatch);
                }
            }
            AcquireResponse::Ack
        }

        fn release(&mut self, connection_id: &str, socket_open: bool) -> Option<String> {
            if let Some(existing) = self.pending_by_connection.get(connection_id) {
                return Some(existing.request_id.clone());
            }
            self.expected.remove(connection_id);
            self.cached
                .retain(|_, (connection, _, _)| connection != connection_id);
            let acquired = self.acquired.remove(connection_id)?;
            if !socket_open {
                return None;
            }
            let request_id = format!("release-{}", self.next_request_id);
            self.next_request_id += 1;
            self.pending_by_connection.insert(
                connection_id.to_string(),
                PendingRelease {
                    connection_id: connection_id.to_string(),
                    request_id: request_id.clone(),
                    runtime: acquired.runtime,
                    state: ReleaseState::Pending,
                },
            );
            self.pending_by_request
                .insert(request_id.clone(), connection_id.to_string());
            Some(request_id)
        }

        fn release_ack(&mut self, request_id: &str) {
            let connection = self
                .pending_by_request
                .get(request_id)
                .expect("pending release");
            let pending = self
                .pending_by_connection
                .get_mut(connection)
                .expect("pending by connection");
            assert_eq!(
                pending.state,
                ReleaseState::Pending,
                "ack must target pending release"
            );
            pending.state = ReleaseState::Resolved;
            let runtime = pending.runtime.clone();
            *self.ack_counts.entry(runtime).or_default() += 1;
            self.pending_by_connection.remove(connection);
            self.pending_by_request.remove(request_id);
        }

        fn release_reject(&mut self, request_id: &str, reason: &str) {
            let connection = self
                .pending_by_request
                .get(request_id)
                .expect("pending release")
                .clone();
            self.failures
                .push(format!("runtime rejected release: {reason}"));
            self.pending_by_request.remove(request_id);
            let runtime = self
                .pending_by_connection
                .remove(&connection)
                .map(|pending| pending.runtime);
            self.expected.remove(&connection);
            if let Some(runtime) = runtime {
                self.session_by_runtime.remove(&runtime);
                self.runtime_by_session.retain(|_, owner| owner != &runtime);
            }
        }

        fn release_timeout(&mut self, request_id: &str) {
            let connection = self
                .pending_by_request
                .get(request_id)
                .expect("pending release")
                .clone();
            self.failures
                .push(format!("release timed out for {connection}"));
            self.pending_by_request.remove(request_id);
            let runtime = self
                .pending_by_connection
                .remove(&connection)
                .map(|pending| pending.runtime);
            self.expected.remove(&connection);
            if let Some(runtime) = runtime {
                self.session_by_runtime.remove(&runtime);
                self.runtime_by_session.retain(|_, owner| owner != &runtime);
            }
        }

        fn send_failure(&mut self, request_id: &str, error: &str) {
            let connection = self
                .pending_by_request
                .get(request_id)
                .expect("pending release")
                .clone();
            self.failures.push(format!("release send failed: {error}"));
            self.pending_by_request.remove(request_id);
            self.pending_by_connection
                .remove(&connection)
                .map(|pending| pending.runtime);
            self.expected.remove(&connection);
        }

        fn runtime_disconnect(&mut self, runtime: &str) {
            self.ack_counts.remove(runtime);
            let affected = self
                .acquired
                .values()
                .filter(|acquired| acquired.runtime == runtime)
                .map(|acquired| acquired.tuple.connection_id.clone())
                .collect::<Vec<_>>();
            for connection in &affected {
                self.expected.remove(connection);
                self.acquired.remove(connection);
            }
            let pending_connections = self
                .pending_by_connection
                .values()
                .filter(|pending| pending.runtime == runtime)
                .map(|pending| pending.connection_id.clone())
                .collect::<Vec<_>>();
            // In the reference model pending releases on a disconnected runtime
            // resolve without an ACK.
            for connection in &pending_connections {
                self.pending_by_connection.remove(connection);
            }
            self.pending_by_request
                .retain(|_, connection| !pending_connections.contains(connection));
            self.cached
                .retain(|_, (_, _, cached_runtime)| cached_runtime != runtime);
            if let Some(session) = self.session_by_runtime.remove(runtime) {
                if self.runtime_by_session.get(&session).map(String::as_str) == Some(runtime) {
                    self.runtime_by_session.remove(&session);
                }
            }
            for connection in affected {
                self.disconnect_handlers_called.push(connection);
            }
        }

        fn pin_count(&self, runtime: &str) -> usize {
            self.acquired
                .values()
                .filter(|acquired| acquired.runtime == runtime)
                .count()
                + self
                    .pending_by_connection
                    .values()
                    .filter(|pending| pending.runtime == runtime)
                    .count()
        }

        fn pending_count(&self) -> usize {
            self.pending_by_connection.len()
        }

        fn ack_count(&self, runtime: &str) -> u64 {
            self.ack_counts.get(runtime).copied().unwrap_or(0)
        }

        fn flush(&mut self) -> Result<(), Vec<String>> {
            if !self.pending_by_connection.is_empty() {
                self.failures
                    .push("flush with unresolved pending release".to_string());
            }
            self.pending_by_connection.clear();
            self.pending_by_request.clear();
            if self.failures.is_empty() {
                Ok(())
            } else {
                Err(std::mem::take(&mut self.failures))
            }
        }
    }

    #[test]
    fn exact_acquire_ack_and_cached_dedupe_keep_single_pin() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());

        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        assert_eq!(ledger.pin_count("r1"), 1);
        // The cached duplicate acquire must return the same ack without creating
        // a second pin.
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        assert_eq!(ledger.pin_count("r1"), 1);
        // Same request id from a different tuple is a conflict.
        assert!(matches!(
            ledger.acquire(
                "r1",
                "acquire-1",
                &tuple.with_build_id("skiff-deployment-artifact-v4:sha256:8")
            ),
            AcquireResponse::Reject(RejectCode::RequestConflict)
        ));
        assert_eq!(ledger.pin_count("r1"), 1);
        assert!(!ledger.fail_stop);
    }

    #[test]
    fn acquire_rejection_codes_are_exact() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        let other = PinTuple::new("c2", "s2");

        // No expectation -> not-acquired.
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Reject(RejectCode::NotAcquired)
        ));

        ledger.expect_connection(tuple.clone());
        // Tuple mismatch.
        assert!(matches!(
            ledger.acquire(
                "r1",
                "acquire-1",
                &tuple
                    .clone()
                    .with_build_id("skiff-deployment-artifact-v4:sha256:8")
            ),
            AcquireResponse::Reject(RejectCode::TupleMismatch)
        ));
        // Exact acquire succeeds and binds the session to r1.
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        // A different runtime cannot claim the same router session.
        assert!(matches!(
            ledger.acquire("r2", "acquire-2", &tuple),
            AcquireResponse::Reject(RejectCode::SenderMismatch)
        ));
        // A second connection on the same runtime with a different session is a
        // sender mismatch too.
        ledger.expect_connection(other.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-3", &other),
            AcquireResponse::Reject(RejectCode::SenderMismatch)
        ));
        // Duplicate expectation is a fail-stop, not a rejection.
        ledger.expect_connection(tuple.clone());
        assert!(ledger.fail_stop);
    }

    #[test]
    fn release_pending_dedupe_ack_and_reject_paths() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));

        let first = ledger.release("c1", true).expect("pending release created");
        let second = ledger
            .release("c1", true)
            .expect("dedupe returns the same pending");
        assert_eq!(first, second);
        assert_eq!(ledger.pending_count(), 1);
        ledger.release_ack(&first);
        assert_eq!(ledger.pending_count(), 0);
        assert_eq!(ledger.ack_count("r1"), 1);
        assert_eq!(ledger.pin_count("r1"), 0);

        // Reject path fails the release and marks the runtime closed.
        // Same runtime session: a runtime connection binds exactly one router
        // session (C-ws §3.2 sender-mismatch).
        let tuple = PinTuple::new("c2", "s1");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-2", &tuple),
            AcquireResponse::Ack
        ));
        let request = ledger.release("c2", true).expect("pending release created");
        ledger.release_reject(&request, "tuple mismatch");
        assert_eq!(ledger.pending_count(), 0);
        assert_eq!(ledger.pin_count("r1"), 0);
        assert_eq!(ledger.failures.len(), 1);

        // Timeout path fails the release and must not leave a pin behind.
        let tuple = PinTuple::new("c3", "s4");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-3", &tuple),
            AcquireResponse::Ack
        ));
        let request = ledger.release("c3", true).expect("pending release created");
        ledger.release_timeout(&request);
        assert_eq!(ledger.pending_count(), 0);
        assert_eq!(ledger.pin_count("r1"), 0);
        assert_eq!(ledger.failures.len(), 2);
    }

    #[test]
    fn release_after_socket_close_resolves_without_pending() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        // Peer close finalizes the connection; the runtime socket is still open
        // in this path, so a pending release is created.
        assert!(ledger.release("c1", true).is_some());
        assert_eq!(ledger.pending_count(), 1);

        // Runtime disconnect resolves the pending release without an ACK and
        // clears every exact pin.
        ledger.runtime_disconnect("r1");
        assert_eq!(ledger.pending_count(), 0);
        assert_eq!(ledger.pin_count("r1"), 0);
        assert_eq!(ledger.ack_count("r1"), 0);
        // The expectation was already removed at release start, so the disconnect
        // handler fires only for connections with a live expectation.
        assert!(ledger.disconnect_handlers_called.is_empty());
    }

    #[test]
    fn disconnect_clears_cached_acquires_and_pending_state() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        assert!(ledger.release("c1", true).is_some());
        ledger.runtime_disconnect("r1");

        // The same request id is no longer cached after disconnect.
        let tuple2 = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple2.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple2),
            AcquireResponse::Ack
        ));
        assert_eq!(ledger.pin_count("r1"), 1);
        assert_eq!(ledger.pending_count(), 0);
    }

    #[test]
    fn flush_aggregates_release_failures() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        let request = ledger.release("c1", true).expect("pending release created");
        ledger.release_timeout(&request);
        let result = ledger.flush();
        assert!(result.is_err(), "flush must surface the release failure");
        assert_eq!(ledger.pending_count(), 0);
        assert_eq!(ledger.pin_count("r1"), 0);
    }

    #[test]
    fn pin_count_covers_acquired_and_pending_release() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        assert_eq!(ledger.pin_count("r1"), 1);
        ledger.release("c1", true);
        assert_eq!(
            ledger.pin_count("r1"),
            1,
            "pending release still pins the runtime"
        );
        let request = ledger
            .pending_by_request
            .keys()
            .next()
            .cloned()
            .expect("pending release request id");
        ledger.release_ack(&request);
        assert_eq!(ledger.pin_count("r1"), 0);
    }

    #[test]
    fn send_failure_does_not_silently_retain_the_pin() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());
        assert!(matches!(
            ledger.acquire("r1", "acquire-1", &tuple),
            AcquireResponse::Ack
        ));
        let request = ledger.release("c1", true).expect("pending release created");
        ledger.send_failure(&request, "writer closed");
        assert_eq!(ledger.pending_count(), 0);
        assert_eq!(ledger.pin_count("r1"), 0);
        assert_eq!(ledger.failures.len(), 1);
    }

    #[test]
    fn fail_stop_flag_is_set_only_for_duplicate_expectation() {
        let mut ledger = LedgerRef::default();
        let tuple = PinTuple::new("c1", "s1");
        ledger.expect_connection(tuple.clone());
        ledger.expect_connection(tuple.with_connection("c1"));
        assert!(ledger.fail_stop);
        assert_eq!(ledger.pending_count(), 0);
        assert_eq!(ledger.pin_count("r1"), 0);
    }
}
