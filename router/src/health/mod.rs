//! Router health projection (batch 12; authority design §3.2/§10).
//!
//! [`HealthAggregator`] is the read-only aggregation owner: it consumes each
//! owner's published snapshot and renders the `/__router/health` wire
//! projection (TS-compatible base shape + §10 counters + `?detail=loop-risk`
//! parity). It never mutates any owner.
//!
pub mod aggregator;
pub mod counters;
pub mod time;
pub mod wire;

pub use aggregator::HealthAggregator;
pub use counters::HealthCounters;
pub use wire::{
    project_capability_connections, project_loop_risk_runtimes, project_replicas, render_base,
    session_facts, ActiveAssemblyProjection, CapabilitiesProjection,
    CapabilityConnectionProjection, LoopRiskDispatcherProjection, LoopRiskHttpStreamProjection,
    LoopRiskProjection, LoopRiskRouterProjection, LoopRiskRuntimeProjection, ReplicaProjection,
    SessionFacts,
};
