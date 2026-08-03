//! Zero-residue gate for the W-actor lane: after
//! success/error/disconnect/saturation/shutdown sequences every owner's
//! pending, tombstone and timer occupancy is zero (authority design §10,
//! E-actor-rust: invocation/control/lease/timer 归零).

mod actor_support;

use std::sync::Arc;

use skiff_router::actor::{
    ActivateInitialControlRequest, ActivationControlPort, ActorActivationBrokerOptions,
    ActorActivationRequestBroker, ActorGetOrCreateRequest, ActorHealthSnapshot,
    ActorInvocationRelay, ActorInvocationRelayOptions, ActorInvokeInput, ActorLeaseExpiryScheduler,
    ActorLogicalKey, ActorOwnerControlBroker, ActorOwnershipRegistry, ControlBrokerOptions,
    IdleEvictControlPort, LeaseSchedulerOptions, OwnerControlRequest, OwnerSettleKind,
};
use skiff_router::actor::{ActorOwnerFence, OwnerReleaseReason};
use skiff_runtime_transport::actor_owner::ActorOwnerControlOperation;

use actor_support::{
    abi, actor_implementation_identity, actor_key, declaration_owner, fence_facts, route_authority,
};

fn key2() -> ActorLogicalKey {
    let mut key = actor_key();
    key.actor_id_hash = format!("sha256:{}", "2".repeat(64));
    key
}

#[derive(Debug, Default)]
struct FakeControl;

impl ActivationControlPort for FakeControl {
    fn send_activate_initial(
        &self,
        _request: &ActivateInitialControlRequest,
    ) -> Result<(), String> {
        Ok(())
    }
}

impl IdleEvictControlPort for FakeControl {
    fn send_idle_evict(
        &self,
        _key: &ActorLogicalKey,
        _fence: &ActorOwnerFence,
        _eviction_request_id: &str,
        _connection: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

fn invoke_input(invocation_id: &str) -> ActorInvokeInput {
    ActorInvokeInput {
        invocation_id: invocation_id.to_string(),
        caller_connection: "conn-caller".to_string(),
        caller_runtime_id: "runtime-caller".to_string(),
        owner_fence: actor_support::fence("runtime-b", 1, 40_000),
        owner_connection: "conn-b".to_string(),
        route_authority: route_authority(),
        correlation: format!("cancel:{invocation_id}"),
        deadline: None,
        test_case_capability: None,
        now: 0,
    }
}

fn get_or_create_request(rpc_id: &str, key: &ActorLogicalKey) -> ActorGetOrCreateRequest {
    ActorGetOrCreateRequest {
        rpc_id: rpc_id.to_string(),
        actor_key: key.clone(),
        actor_abi_identity: abi(),
        actor_implementation_identity: actor_implementation_identity(),
        declaration_owner: declaration_owner(),
        bootstrap_bytes: vec![0x0a],
        owner_runtime_id: "runtime-b".to_string(),
        owner_connection: "conn-b".to_string(),
        route_authority: route_authority(),
        deadline: None,
        test_case_capability: None,
        test_case_parent_request_id: None,
        now: 0,
    }
}

fn assert_all_zero(health: &ActorHealthSnapshot) {
    assert_eq!(health.ownership.current_fences, 0, "current fences");
    assert_eq!(health.ownership.in_flight_reservations, 0, "reservations");
    assert_eq!(health.activation.pending_claims, 0, "activation claims");
    assert_eq!(health.activation.tombstones, 0, "activation tombstones");
    assert_eq!(health.invocation.pending, 0, "invocation pending");
    assert_eq!(health.invocation.tombstones, 0, "invocation tombstones");
    assert_eq!(health.control.pending, 0, "control pending");
    assert_eq!(health.control.tombstones, 0, "control tombstones");
    assert_eq!(health.lease.eviction_pending, 0, "lease eviction pending");
    assert_eq!(health.spawn.capacity_in_use, 0, "spawn capacity");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_error_disconnect_and_shutdown_leave_every_owner_zero() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let control_broker =
            Arc::new(ActorOwnerControlBroker::new(ControlBrokerOptions::default()));
        let control_port: Arc<dyn ActivationControlPort> = Arc::new(FakeControl);
        let activation = ActorActivationRequestBroker::new(
            Arc::clone(&registry),
            control_port,
            ActorActivationBrokerOptions::default(),
        );
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        let scheduler = ActorLeaseExpiryScheduler::new(
            Arc::clone(&registry),
            Arc::new(FakeControl),
            LeaseSchedulerOptions::default(),
        );
        let key = actor_key();

        // Success path: claim -> commit -> resolve.
        let outcome = activation.get_or_create(&get_or_create_request("rpc:ok", &key));
        let request_id = match outcome {
            skiff_router::actor::GetOrCreateOutcome::StartedActivation { request_id } => request_id,
            other => panic!("unexpected outcome {other:?}"),
        };
        activation.on_activation_ack(&request_id, "runtime-b", "conn-b", true, 1000);
        assert_eq!(
            activation.outcome_for("rpc:ok").as_deref(),
            Some("resolved:1")
        );
        registry
            .release(
                &key,
                &registry.current_owner(&key).expect("committed owner"),
                OwnerReleaseReason::Shutdown,
            )
            .expect("release success-path owner");

        // Error path: timeout aborts a second claim (epoch mismatch is
        // impossible while owner held; force a fresh key via incarnation).
        let second_key = key2();
        let outcome = activation.get_or_create(&get_or_create_request("rpc:timeout", &second_key));
        let request_id = match outcome {
            skiff_router::actor::GetOrCreateOutcome::StartedActivation { request_id } => request_id,
            other => panic!("unexpected outcome {other:?}"),
        };
        activation.on_activation_timeout(&request_id, 31_000);
        assert_eq!(
            activation.outcome_for("rpc:timeout").as_deref(),
            Some("failed:ActivationTimeout")
        );

        // Invocation: admit 3, settle 1, deadline 1, owner disconnect 1.
        relay.invoke(&invoke_input("inv:1")).expect("invoke 1");
        let mut deadline_input = invoke_input("inv:2");
        deadline_input.deadline = Some(
            skiff_runtime_transport::actor_method::ActorMethodDeadlineFrameHeader {
                timeout_ms: 100,
                expires_at: "deadline".to_string(),
            },
        );
        relay
            .invoke(&deadline_input)
            .expect("invoke 2 with deadline");
        relay.invoke(&invoke_input("inv:3")).expect("invoke 3");
        let fence = actor_support::fence("runtime-b", 1, 40_000);
        relay
            .on_owner_settle("inv:1", &fence, "conn-b", OwnerSettleKind::Return)
            .expect("settle inv:1");
        relay.expire_deadlines(100);
        relay.on_owner_disconnect("runtime-b", "conn-b");
        assert_eq!(relay.health().pending, 0);

        // Control: 2 pending; ACK one, timeout the other.
        for (request_id, operation) in [
            ("control:1", ActorOwnerControlOperation::ActivateInitial),
            ("control:2", ActorOwnerControlOperation::IdleEvict),
        ] {
            control_broker
                .send_control(&OwnerControlRequest {
                    request_id: request_id.to_string(),
                    operation,
                    runtime_id: "runtime-b".to_string(),
                    connection: "conn-b".to_string(),
                    fence: fence.clone(),
                    route_authority: route_authority(),
                    deadline_at: 10_000,
                })
                .expect("send control");
        }
        control_broker.on_ack(
            "runtime-b",
            "control:1",
            ActorOwnerControlOperation::ActivateInitial,
            "conn-b",
            true,
        );
        control_broker.timeout("control:2");
        assert_eq!(control_broker.health().pending, 0);

        // Lease: mint an eviction on the second key, then shutdown clears it.
        let lease_key = key2();
        registry.ensure_present(
            &lease_key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
        );
        let token = registry
            .reserve(&lease_key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        registry
            .commit(&token, &fence_facts(), 0, 40_000)
            .expect("commit");
        scheduler.mark_live(&lease_key, 0, "conn-b");
        scheduler.sweep(30_000);
        assert_eq!(scheduler.health().eviction_pending, 1);
        scheduler.shutdown();
        registry
            .release(
                &lease_key,
                &registry.current_owner(&lease_key).expect("owner"),
                OwnerReleaseReason::Shutdown,
            )
            .expect("release");
        assert_eq!(registry.health().current_fences, 0);

        // Shutdown clears remaining broker/relay residue.
        activation.shutdown();
        relay.shutdown();
        control_broker.shutdown();

        let health = ActorHealthSnapshot {
            catalog: Default::default(),
            ownership: registry.health(),
            activation: activation.health(),
            invocation: relay.health(),
            control: control_broker.health(),
            lease: scheduler.health(),
            spawn: Default::default(),
        };
        assert_all_zero(&health);
    }

    #[test]
    fn saturation_paths_fail_closed_and_return_to_zero() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let activation = ActorActivationRequestBroker::new(
            Arc::clone(&registry),
            Arc::new(FakeControl),
            ActorActivationBrokerOptions {
                max_claims: 1,
                ..ActorActivationBrokerOptions::default()
            },
        );
        activation.get_or_create(&get_or_create_request("rpc:1", &actor_key()));
        activation.get_or_create(&get_or_create_request("rpc:2", &key2()));
        assert_eq!(activation.health().saturated, 1);
        activation.shutdown();
        assert_eq!(activation.health().pending_claims, 0);
        assert_eq!(activation.health().tombstones, 0);
        assert_eq!(registry.health().in_flight_reservations, 0);
        assert_eq!(registry.health().current_fences, 0);

        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions {
            max_concurrency: 1,
            ..ActorInvocationRelayOptions::default()
        });
        relay.invoke(&invoke_input("inv:1")).expect("invoke");
        assert!(relay.invoke(&invoke_input("inv:2")).is_err());
        relay.shutdown();
        assert_eq!(relay.health().pending, 0);
        assert_eq!(relay.health().tombstones, 0);

        let control = ActorOwnerControlBroker::new(ControlBrokerOptions {
            max_pending: 1,
            ..ControlBrokerOptions::default()
        });
        control
            .send_control(&OwnerControlRequest {
                request_id: "control:1".to_string(),
                operation: ActorOwnerControlOperation::ActivateInitial,
                runtime_id: "runtime-b".to_string(),
                connection: "conn-b".to_string(),
                fence: actor_support::fence("runtime-b", 1, 40_000),
                route_authority: route_authority(),
                deadline_at: 10_000,
            })
            .expect("send");
        assert!(control
            .send_control(&OwnerControlRequest {
                request_id: "control:2".to_string(),
                operation: ActorOwnerControlOperation::IdleEvict,
                runtime_id: "runtime-b".to_string(),
                connection: "conn-b".to_string(),
                fence: actor_support::fence("runtime-b", 1, 40_000),
                route_authority: route_authority(),
                deadline_at: 10_000,
            })
            .is_err());
        control.shutdown();
        assert_eq!(control.health().pending, 0);
        assert_eq!(control.health().tombstones, 0);
    }

    #[test]
    fn disconnect_paths_leave_all_owners_zero() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let activation = ActorActivationRequestBroker::new(
            Arc::clone(&registry),
            Arc::new(FakeControl),
            ActorActivationBrokerOptions::default(),
        );
        let outcome = activation.get_or_create(&get_or_create_request("rpc:1", &actor_key()));
        let request_id = match outcome {
            skiff_router::actor::GetOrCreateOutcome::StartedActivation { request_id } => request_id,
            other => panic!("unexpected outcome {other:?}"),
        };
        activation.on_owner_disconnect("runtime-b", "conn-b");
        assert_eq!(activation.health().pending_claims, 1);
        activation.on_activation_timeout(&request_id, 31_000);
        assert_eq!(activation.health().pending_claims, 0);
        activation.shutdown();

        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        relay.invoke(&invoke_input("inv:1")).expect("invoke");
        relay.invoke(&invoke_input("inv:2")).expect("invoke");
        relay.on_owner_disconnect("runtime-b", "conn-b");
        assert_eq!(relay.health().pending, 0);
        relay.shutdown();
        assert_eq!(relay.health().tombstones, 0);

        let control = ActorOwnerControlBroker::new(ControlBrokerOptions::default());
        control
            .send_control(&OwnerControlRequest {
                request_id: "control:1".to_string(),
                operation: ActorOwnerControlOperation::ActivateInitial,
                runtime_id: "runtime-b".to_string(),
                connection: "conn-b".to_string(),
                fence: actor_support::fence("runtime-b", 1, 40_000),
                route_authority: route_authority(),
                deadline_at: 10_000,
            })
            .expect("send");
        control.on_owner_disconnect("runtime-b", "conn-b");
        assert_eq!(control.health().pending, 0);
        control.shutdown();
        assert_eq!(control.health().tombstones, 0);
    }
}
