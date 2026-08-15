//! TEST-ONLY reference model for `WebSocketRequestBroker`
//! (C-ws §4, `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-ws-contract.md`).
//!
//! Mirrors the canonical TS `WebSocketRequestBroker` semantics: outbound/
//! inbound peer correlation, deadline, tombstone FIFO/TTL/capacity,
//! runtime cancel/disconnect, captured writer fence and generation close.
//! Not production code.

// This standalone integration-test crate is compiled only as a test target;
// wrapping the whole file in `cfg(test)` would add indentation without scope.
#![allow(clippy::tests_outside_test_module)]

use std::collections::HashMap;

const OUTBOUND_PER_GENERATION: usize = 2;
const INBOUND_PER_GENERATION: usize = 2;
const TOMBSTONE_CAPACITY: usize = 2;

#[derive(Debug, Clone)]
struct PeerWriter {
    writes: Vec<String>,
    fail_next: bool,
    close: Option<(u16, String)>,
}

impl PeerWriter {
    fn new() -> Self {
        Self {
            writes: Vec::new(),
            fail_next: false,
            close: None,
        }
    }

    fn write(&mut self, frame: String) -> bool {
        if self.fail_next {
            self.fail_next = false;
            return false;
        }
        self.writes.push(frame);
        true
    }
}

#[derive(Debug, Clone)]
struct Generation {
    socket_generation: String,
    writer: PeerWriter,
    open: bool,
    outbound_active: usize,
    inbound_active: usize,
    sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeOutcome {
    Success,
    ConnectionUnavailable,
    TransportUnavailable,
    ProtocolError,
    ResourceLimit,
}

#[derive(Debug, Clone)]
struct RuntimeSource {
    sender: &'static str,
    session: &'static str,
}

struct BrokerRef {
    generations: HashMap<String, Generation>,
    outbound_by_peer: HashMap<String, String>,
    outbound_by_runtime: HashMap<(String, String, String), String>,
    inbound_by_peer: HashMap<String, String>,
    outbound_tombstones: Vec<String>,
    inbound_tombstones: Vec<String>,
    protocol_violations: Vec<String>,
    outbound_per_generation: usize,
    inbound_per_generation: usize,
    tombstone_capacity: usize,
}

impl BrokerRef {
    fn default() -> Self {
        Self {
            generations: HashMap::new(),
            outbound_by_peer: HashMap::new(),
            outbound_by_runtime: HashMap::new(),
            inbound_by_peer: HashMap::new(),
            outbound_tombstones: Vec::new(),
            inbound_tombstones: Vec::new(),
            protocol_violations: Vec::new(),
            outbound_per_generation: OUTBOUND_PER_GENERATION,
            inbound_per_generation: INBOUND_PER_GENERATION,
            tombstone_capacity: TOMBSTONE_CAPACITY,
        }
    }

    fn with_limits(
        outbound_per_generation: usize,
        inbound_per_generation: usize,
        tombstone_capacity: usize,
    ) -> Self {
        Self {
            outbound_per_generation,
            inbound_per_generation,
            tombstone_capacity,
            ..Self::default()
        }
    }

    fn attach(&mut self, connection: &str, socket_generation: &str) -> String {
        let gen_key = format!("{connection}\0{socket_generation}");
        if self.generations.contains_key(&gen_key) {
            panic!("generation {gen_key} already attached");
        }
        self.generations.insert(
            gen_key.clone(),
            Generation {
                socket_generation: socket_generation.to_string(),
                writer: PeerWriter::new(),
                open: true,
                outbound_active: 0,
                inbound_active: 0,
                sequence: 0,
            },
        );
        gen_key
    }

    fn writer(&mut self, gen_key: &str) -> &mut PeerWriter {
        &mut self
            .generations
            .get_mut(gen_key)
            .expect("generation")
            .writer
    }

    fn handle_runtime_request(
        &mut self,
        gen_key: &str,
        request_id: &str,
        source: &RuntimeSource,
    ) -> RuntimeOutcome {
        let Some(generation) = self.generations.get_mut(gen_key) else {
            return RuntimeOutcome::ConnectionUnavailable;
        };
        if !generation.open {
            return RuntimeOutcome::ConnectionUnavailable;
        }
        let runtime_key = (
            source.sender.to_string(),
            source.session.to_string(),
            request_id.to_string(),
        );
        if self.outbound_by_runtime.contains_key(&runtime_key) {
            self.protocol_violations.push(format!(
                "duplicate connection.request correlation {request_id}"
            ));
            return RuntimeOutcome::ProtocolError;
        }
        if generation.outbound_active >= self.outbound_per_generation {
            return RuntimeOutcome::ResourceLimit;
        }
        let peer_key = format!("{}:{}", generation.socket_generation, generation.sequence);
        generation.sequence += 1;
        let frame = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":\"{peer_key}\",\"method\":\"chat.send\",\"params\":{{}}}}"
        );
        if !generation.writer.write(frame) {
            return RuntimeOutcome::TransportUnavailable;
        }
        generation.outbound_active += 1;
        self.outbound_by_peer
            .insert(peer_key.clone(), gen_key.to_string());
        self.outbound_by_runtime.insert(runtime_key, peer_key);
        RuntimeOutcome::Success
    }

    fn settle_outbound(&mut self, gen_key: &str, peer_key: &str) -> bool {
        let Some(owner) = self.outbound_by_peer.remove(peer_key) else {
            return false;
        };
        if owner != gen_key {
            self.outbound_by_peer.insert(peer_key.to_string(), owner);
            return false;
        }
        self.outbound_by_runtime
            .retain(|_, value| value != peer_key);
        let generation = self.generations.get_mut(gen_key).expect("generation");
        generation.outbound_active -= 1;
        self.outbound_tombstones.push(peer_key.to_string());
        evict_tombstones(&mut self.outbound_tombstones, self.tombstone_capacity);
        true
    }

    fn peer_response(&mut self, gen_key: &str, peer_key: &str) -> Result<(), (u16, String)> {
        if self.settle_outbound(gen_key, peer_key) {
            return Ok(());
        }
        if self.outbound_tombstones.contains(&peer_key.to_string()) {
            // Late response isolated by the tombstone fence.
            return Ok(());
        }
        Err((1002, "unknown JSON-RPC response id".to_string()))
    }

    fn deadline(&mut self, gen_key: &str, request_id: &str, source: &RuntimeSource) {
        let runtime_key = (
            source.sender.to_string(),
            source.session.to_string(),
            request_id.to_string(),
        );
        if let Some(peer_key) = self.outbound_by_runtime.get(&runtime_key).cloned() {
            self.settle_outbound(gen_key, &peer_key);
        }
    }

    fn runtime_cancel(&mut self, source: &RuntimeSource, request_id: &str) -> bool {
        let runtime_key = (
            source.sender.to_string(),
            source.session.to_string(),
            request_id.to_string(),
        );
        let Some(peer_key) = self.outbound_by_runtime.get(&runtime_key).cloned() else {
            return false;
        };
        let gen_key = self
            .outbound_by_peer
            .get(&peer_key)
            .cloned()
            .expect("owner");
        self.settle_outbound(&gen_key, &peer_key);
        true
    }

    fn runtime_disconnect(&mut self, source: &RuntimeSource) -> usize {
        let affected = self
            .outbound_by_runtime
            .iter()
            .filter(|((sender, session, _), _)| {
                sender == source.sender && session == source.session
            })
            .map(|(_, peer_key)| peer_key.clone())
            .collect::<Vec<_>>();
        let count = affected.len();
        for peer_key in &affected {
            let gen_key = self.outbound_by_peer.get(peer_key).cloned().expect("owner");
            self.settle_outbound(&gen_key, peer_key);
        }
        count
    }

    fn peer_request(&mut self, gen_key: &str, peer_id: &str) -> Result<(), (u16, String)> {
        if self.inbound_by_peer.contains_key(peer_id)
            || self.inbound_tombstones.contains(&peer_id.to_string())
        {
            return Err((1002, "duplicate JSON-RPC request id".to_string()));
        }
        let generation = self.generations.get_mut(gen_key).expect("generation");
        if !generation.open || generation.inbound_active >= self.inbound_per_generation {
            self.inbound_tombstones.push(peer_id.to_string());
            evict_tombstones(&mut self.inbound_tombstones, self.tombstone_capacity);
            return Ok(());
        }
        generation.inbound_active += 1;
        self.inbound_by_peer
            .insert(peer_id.to_string(), gen_key.to_string());
        Ok(())
    }

    fn inbound_dispatch(&mut self, gen_key: &str, peer_id: &str) {
        if self.inbound_by_peer.remove(peer_id).as_deref() != Some(gen_key) {
            panic!("inbound dispatch for unknown peer id {peer_id}");
        }
        self.generations
            .get_mut(gen_key)
            .expect("generation")
            .inbound_active -= 1;
        self.inbound_tombstones.push(peer_id.to_string());
        evict_tombstones(&mut self.inbound_tombstones, self.tombstone_capacity);
    }

    fn close_generation(&mut self, gen_key: &str, code: u16, reason: &str) {
        let Some(generation) = self.generations.get_mut(gen_key) else {
            return;
        };
        if !generation.open {
            return;
        }
        generation.open = false;
        generation.outbound_active = 0;
        generation.inbound_active = 0;
        generation.writer.close = Some((code, reason.to_string()));
        let owner = gen_key.to_string();
        let _ = generation;
        self.outbound_by_peer.retain(|_, value| value != &owner);
        self.outbound_by_runtime.retain(|_, value| {
            self.outbound_by_peer
                .get(value)
                .is_some_and(|value| value == &owner)
        });
        self.inbound_by_peer.retain(|_, value| value != &owner);
        self.outbound_tombstones
            .retain(|peer_key| self.outbound_by_peer.contains_key(peer_key));
        self.inbound_tombstones
            .retain(|peer_id| self.inbound_by_peer.contains_key(peer_id));
        self.generations.remove(gen_key);
    }

    fn peer_disconnect(&mut self, gen_key: &str) {
        let peers = {
            let Some(generation) = self.generations.get_mut(gen_key) else {
                return;
            };
            if !generation.open {
                return;
            }
            generation.open = false;
            let peers = self
                .outbound_by_peer
                .iter()
                .filter(|(_, owner)| *owner == gen_key)
                .map(|(peer_key, _)| peer_key.clone())
                .collect::<Vec<_>>();
            let inbound = self
                .inbound_by_peer
                .iter()
                .filter(|(_, owner)| *owner == gen_key)
                .map(|(peer_id, _)| peer_id.clone())
                .collect::<Vec<_>>();
            for peer_id in &inbound {
                self.inbound_by_peer.remove(peer_id);
            }
            generation.inbound_active = 0;
            peers
        };
        for peer_key in &peers {
            self.settle_outbound(gen_key, peer_key);
        }
        // TS `closeGeneration` removes the generation's tombstones after
        // detaching every entry.
        self.outbound_tombstones
            .retain(|peer_key| self.outbound_by_peer.contains_key(peer_key));
        self.inbound_tombstones
            .retain(|peer_id| self.inbound_by_peer.contains_key(peer_id));
        self.generations.remove(gen_key);
    }

    fn outbound_pending(&self) -> usize {
        self.generations
            .values()
            .map(|generation| generation.outbound_active)
            .sum()
    }

    fn inbound_pending(&self) -> usize {
        self.generations
            .values()
            .map(|generation| generation.inbound_active)
            .sum()
    }
}

fn evict_tombstones(tombstones: &mut Vec<String>, capacity: usize) {
    while tombstones.len() > capacity {
        tombstones.remove(0);
    }
}

fn source(sender: &'static str, session: &'static str) -> RuntimeSource {
    RuntimeSource { sender, session }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_roundtrip_settles_exact_runtime_source() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::Success
        );
        assert_eq!(broker.outbound_pending(), 1);
        assert_eq!(broker.writer(&gen).writes.len(), 1);
        assert!(broker.writer(&gen).writes[0].contains("\"id\":\"g1:0\""));

        assert!(broker.peer_response(&gen, "g1:0").is_ok());
        assert_eq!(broker.outbound_pending(), 0);
        assert_eq!(broker.outbound_tombstones, vec!["g1:0"]);
    }

    #[test]
    fn out_of_order_responses_and_late_response_isolation() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-2", &runtime),
            RuntimeOutcome::Success
        );
        assert!(broker.peer_response(&gen, "g1:1").is_ok());
        assert!(broker.peer_response(&gen, "g1:0").is_ok());
        assert_eq!(broker.outbound_pending(), 0);
        // Late response for a settled id is isolated by the tombstone.
        assert!(broker.peer_response(&gen, "g1:1").is_ok());
        assert!(
            broker.generations.contains_key(&gen),
            "generation stays open"
        );
    }

    #[test]
    fn deadline_wins_exactly_once() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::Success
        );
        broker.deadline(&gen, "req-1", &runtime);
        assert_eq!(broker.outbound_pending(), 0);
        assert_eq!(broker.outbound_tombstones.len(), 1);
        // Late peer response after the deadline is isolated.
        assert!(broker.peer_response(&gen, "g1:0").is_ok());
        assert!(
            broker.generations.contains_key(&gen),
            "deadline does not close the generation"
        );
    }

    #[test]
    fn runtime_cancel_detaches_without_peer_write() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::Success
        );
        assert!(broker.runtime_cancel(&runtime, "req-1"));
        assert_eq!(broker.outbound_pending(), 0);
        assert_eq!(broker.outbound_tombstones.len(), 1);
        assert_eq!(
            broker.writer(&gen).writes.len(),
            1,
            "cancel must not write a peer frame"
        );
    }

    #[test]
    fn runtime_disconnect_detaches_only_that_session() {
        let mut broker = BrokerRef::default();
        let gen_a = broker.attach("c1", "g1");
        let gen_b = broker.attach("c2", "g2");
        let runtime_a = source("r1", "s1");
        let runtime_b = source("r2", "s2");
        assert_eq!(
            broker.handle_runtime_request(&gen_a, "req-a", &runtime_a),
            RuntimeOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request(&gen_b, "req-b", &runtime_b),
            RuntimeOutcome::Success
        );
        assert_eq!(broker.runtime_disconnect(&runtime_a), 1);
        assert_eq!(broker.outbound_pending(), 1);
        assert_eq!(broker.outbound_tombstones.len(), 1);
        assert!(broker.peer_response(&gen_b, "g2:0").is_ok());
        assert_eq!(broker.outbound_pending(), 0);
    }

    #[test]
    fn capacity_and_duplicate_runtime_key_fail_closed() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-2", &runtime),
            RuntimeOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-3", &runtime),
            RuntimeOutcome::ResourceLimit
        );
        assert_eq!(
            broker.writer(&gen).writes.len(),
            2,
            "capacity rejection must not write"
        );
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::ProtocolError
        );
        assert_eq!(broker.protocol_violations.len(), 1);
    }

    #[test]
    fn duplicate_inbound_id_and_unknown_response_close_generation() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        assert!(broker.peer_request(&gen, "p1").is_ok());
        broker.inbound_dispatch(&gen, "p1");
        assert_eq!(broker.inbound_pending(), 0);
        assert_eq!(broker.inbound_tombstones, vec!["p1"]);
        assert!(broker.peer_request(&gen, "p1").is_err());
        broker.close_generation(&gen, 1002, "duplicate JSON-RPC request id");
        assert!(!broker.generations.contains_key(&gen));
        assert_eq!(broker.outbound_tombstones.len(), 0);
        assert_eq!(broker.inbound_tombstones.len(), 0);

        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let error = broker
            .peer_response(&gen, "unknown:0")
            .expect_err("unknown response id must close");
        assert_eq!(error.0, 1002);
    }

    #[test]
    fn peer_disconnect_settles_all_pending_and_removes_generation() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::Success
        );
        assert!(broker.peer_request(&gen, "p1").is_ok());
        broker.peer_disconnect(&gen);
        assert_eq!(broker.outbound_pending(), 0);
        assert_eq!(broker.inbound_pending(), 0);
        assert_eq!(broker.outbound_by_peer.len(), 0);
        assert_eq!(broker.inbound_by_peer.len(), 0);
        assert!(!broker.generations.contains_key(&gen));
        // Tombstones were removed with the generation; a late response now has
        // nowhere to land and is treated as unknown by the endpoint owner.
        assert_eq!(broker.outbound_tombstones.len(), 0);
    }

    #[test]
    fn writer_failure_fences_only_the_exact_request() {
        let mut broker = BrokerRef::default();
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        broker.writer(&gen).fail_next = true;
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-1", &runtime),
            RuntimeOutcome::TransportUnavailable
        );
        assert_eq!(broker.outbound_pending(), 0);
        assert_eq!(broker.writer(&gen).writes.len(), 0);
        assert_eq!(
            broker.handle_runtime_request(&gen, "req-2", &runtime),
            RuntimeOutcome::Success
        );
        assert_eq!(broker.outbound_pending(), 1);
        assert_eq!(broker.writer(&gen).writes.len(), 1);
    }

    #[test]
    fn tombstone_fifo_eviction_permits_reuse_but_keeps_active_fence() {
        let mut broker = BrokerRef::with_limits(3, 2, 2);
        let gen = broker.attach("c1", "g1");
        let runtime = source("r1", "s1");
        for request_id in ["req-1", "req-2", "req-3"] {
            assert_eq!(
                broker.handle_runtime_request(&gen, request_id, &runtime),
                RuntimeOutcome::Success
            );
        }
        assert_eq!(broker.outbound_tombstones.len(), 0);
        assert!(broker.peer_response(&gen, "g1:0").is_ok());
        assert!(broker.peer_response(&gen, "g1:1").is_ok());
        assert!(broker.peer_response(&gen, "g1:2").is_ok());
        assert_eq!(broker.outbound_tombstones.len(), 2);
        // The first tombstone was evicted by FIFO; its late response is no longer
        // isolated and must not reopen state.
        assert_eq!(broker.peer_response(&gen, "g1:0").unwrap_err().0, 1002);
        assert_eq!(broker.outbound_pending(), 0);
    }
}
