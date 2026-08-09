use std::fmt;

use skiff_artifact_model::{PackageArtifactRef, ServiceDeploymentRef, ServiceRequirementKey};

/// Bounded resource whose configured link ceiling was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BytecodeLinkLimit {
    Packages,
    RootSpecializations,
    Specializations,
    CodeWordsPerFunction,
    TotalCodeWords,
    RelocationsPerFunction,
    TotalRelocations,
    ImageTableEntries,
    TotalImageTableEntries,
    TotalFunctionTableEntries,
    TypeNestingDepth,
    ExpandedTypeNodes,
    ExpandedTypeBytes,
    ConstantGraphNodes,
    ConstantGraphEdges,
}

impl BytecodeLinkLimit {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::RootSpecializations => "root specializations",
            Self::Specializations => "specializations",
            Self::CodeWordsPerFunction => "code words per function",
            Self::TotalCodeWords => "total code words",
            Self::RelocationsPerFunction => "relocations per function",
            Self::TotalRelocations => "total relocations",
            Self::ImageTableEntries => "image table entries",
            Self::TotalImageTableEntries => "total image table entries",
            Self::TotalFunctionTableEntries => "total function table entries",
            Self::TypeNestingDepth => "type nesting depth",
            Self::ExpandedTypeNodes => "expanded concrete type nodes",
            Self::ExpandedTypeBytes => "expanded concrete type bytes",
            Self::ConstantGraphNodes => "constant graph nodes",
            Self::ConstantGraphEdges => "constant graph edges",
        }
    }
}

/// Linker-owned fact family that must be established before a candidate can
/// be emitted.
///
/// These are construction obligations, not semantic-verifier proofs. The
/// verifier independently recomputes safety facts and may reject every field
/// emitted by the linker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BytecodeLinkObligation {
    ExactPackageClosure,
    CanonicalRootSet,
    ConcreteSpecialization,
    FrameAndValueTransferPlan,
    CallableEffectPlan,
    RelocationResolution,
    ConcreteTargetTables,
    ConcreteTypeAndShapeTables,
    ExceptionAndResumePlan,
    CallbackCapturePlan,
    ConstantInitializationPlan,
    SourceAndStatementTables,
    ControlFlowAndStackMap,
    CandidateAssembly,
}

impl BytecodeLinkObligation {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ExactPackageClosure => "exact package closure",
            Self::CanonicalRootSet => "canonical root set",
            Self::ConcreteSpecialization => "concrete specialization",
            Self::FrameAndValueTransferPlan => "frame and value-transfer plan",
            Self::CallableEffectPlan => "callable effect plan",
            Self::RelocationResolution => "relocation resolution",
            Self::ConcreteTargetTables => "concrete target tables",
            Self::ConcreteTypeAndShapeTables => "concrete type and shape tables",
            Self::ExceptionAndResumePlan => "exception and resume plan",
            Self::CallbackCapturePlan => "callback capture plan",
            Self::ConstantInitializationPlan => "constant initialization plan",
            Self::SourceAndStatementTables => "source and statement tables",
            Self::ControlFlowAndStackMap => "control flow and stack map",
            Self::CandidateAssembly => "candidate assembly",
        }
    }
}

/// Stable source location attached to a deployment-link failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BytecodeLinkLocation {
    Deployment {
        deployment: Box<ServiceDeploymentRef>,
    },
    Package {
        package: Box<PackageArtifactRef>,
    },
    ServiceDependency {
        key: ServiceRequirementKey,
    },
    Function {
        package: Box<PackageArtifactRef>,
        function_key: String,
    },
    Instruction {
        package: Box<PackageArtifactRef>,
        function_key: String,
        artifact_pc: u32,
    },
    Relocation {
        package: Box<PackageArtifactRef>,
        function_key: String,
        relocation_index: u32,
    },
    Constant {
        package: Box<PackageArtifactRef>,
        node_index: u32,
    },
}

impl fmt::Display for BytecodeLinkLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deployment { deployment } => {
                write!(
                    formatter,
                    "deployment {}",
                    deployment.deployment_artifact_identity
                )
            }
            Self::Package { package } => {
                write!(formatter, "package {}", package.package_build_id)
            }
            Self::ServiceDependency { key } => write!(
                formatter,
                "service dependency ({}, slot {})",
                key.caller_package_build_id, key.service_requirement_slot
            ),
            Self::Function {
                package,
                function_key,
            } => write!(
                formatter,
                "package {} function {function_key}",
                package.package_build_id
            ),
            Self::Instruction {
                package,
                function_key,
                artifact_pc,
            } => write!(
                formatter,
                "package {} function {function_key} pc {artifact_pc}",
                package.package_build_id
            ),
            Self::Relocation {
                package,
                function_key,
                relocation_index,
            } => write!(
                formatter,
                "package {} function {function_key} relocation {relocation_index}",
                package.package_build_id
            ),
            Self::Constant {
                package,
                node_index,
            } => write!(
                formatter,
                "package {} constant node {node_index}",
                package.package_build_id
            ),
        }
    }
}

/// Structured, fail-closed result of deployment bytecode linking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BytecodeLinkError {
    LimitExceeded {
        limit: BytecodeLinkLimit,
        actual: u64,
        max: u64,
        location: BytecodeLinkLocation,
    },
    UnsatisfiedObligation {
        obligation: BytecodeLinkObligation,
        location: BytecodeLinkLocation,
        detail: String,
    },
    /// The crate has not implemented a required link obligation.
    ///
    /// This is never a soft warning and must never publish a candidate or
    /// trigger legacy/tree fallback.
    ImplementationUnavailable {
        obligation: BytecodeLinkObligation,
        location: BytecodeLinkLocation,
    },
}

impl fmt::Display for BytecodeLinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded {
                limit,
                actual,
                max,
                location,
            } => write!(
                formatter,
                "bytecode link limit {} exceeded at {location}: actual {actual} > max {max}",
                limit.name()
            ),
            Self::UnsatisfiedObligation {
                obligation,
                location,
                detail,
            } => write!(
                formatter,
                "bytecode {} linking failed at {location}: {detail}",
                obligation.name()
            ),
            Self::ImplementationUnavailable {
                obligation,
                location,
            } => write!(
                formatter,
                "bytecode {} linking is unavailable at {location}",
                obligation.name()
            ),
        }
    }
}

impl std::error::Error for BytecodeLinkError {}
