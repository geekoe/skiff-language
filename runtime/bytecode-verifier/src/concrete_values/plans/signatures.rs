use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedBytecodeCandidate, LinkedCallableSignature, LinkedInterfaceTableKind,
    LinkedNativeCallableSignature, LinkedValueTransferPlan, TypeIndex,
};

use super::super::ConcreteValueFacts;
use super::{prove_position, table_location};
use crate::{VerificationError, VerificationLocation};

pub(super) fn prove_signature_plans(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
) -> Result<(), VerificationError> {
    for (row, entry) in candidate.operation_entries().iter().enumerate() {
        prove_callable_signature(
            facts,
            entry.signature(),
            table_location(CandidateTable::OperationEntries, row)?,
            "operation signature",
        )?;
    }
    for (row, entry) in candidate.gateway_entries().iter().enumerate() {
        let location = table_location(CandidateTable::GatewayEntries, row)?;
        for (ordinal, callable) in entry.callables().iter().enumerate() {
            prove_callable_signature(
                facts,
                callable.signature(),
                location,
                &format!(
                    "gateway callable ordinal {ordinal} ({:?}) signature",
                    callable.role()
                ),
            )?;
        }
    }
    for target in candidate.service_operations() {
        prove_callable_signature(
            facts,
            target.signature(),
            VerificationLocation::Table {
                table: CandidateTable::ServiceOperations,
                row: target.index().get(),
            },
            "service operation signature",
        )?;
    }
    for target in candidate.actor_creates() {
        prove_callable_signature(
            facts,
            target.signature(),
            VerificationLocation::Table {
                table: CandidateTable::ActorCreates,
                row: target.index().get(),
            },
            "actor create signature",
        )?;
    }
    for target in candidate.actor_methods() {
        prove_callable_signature(
            facts,
            target.signature(),
            VerificationLocation::Table {
                table: CandidateTable::ActorMethods,
                row: target.index().get(),
            },
            "actor method signature",
        )?;
    }
    for table in candidate.interface_tables() {
        let location = VerificationLocation::Table {
            table: CandidateTable::InterfaceTables,
            row: table.index().get(),
        };
        match table.kind() {
            LinkedInterfaceTableKind::Requirement(requirement) => {
                for (ordinal, method) in requirement.methods().iter().enumerate() {
                    prove_callable_signature(
                        facts,
                        method.signature(),
                        location,
                        &format!("interface requirement method ordinal {ordinal} signature"),
                    )?;
                }
            }
            LinkedInterfaceTableKind::Callback(callback) => {
                for (ordinal, method) in callback.methods().iter().enumerate() {
                    prove_callable_signature(
                        facts,
                        method.signature(),
                        location,
                        &format!("interface callback method ordinal {ordinal} signature"),
                    )?;
                }
            }
            LinkedInterfaceTableKind::Local(local) => {
                for (ordinal, method) in local.methods().iter().enumerate() {
                    prove_callable_signature(
                        facts,
                        method.signature(),
                        location,
                        &format!("local interface method ordinal {ordinal} signature"),
                    )?;
                }
            }
            LinkedInterfaceTableKind::Remote(remote) => {
                for (ordinal, method) in remote.methods().iter().enumerate() {
                    prove_callable_signature(
                        facts,
                        method.signature(),
                        location,
                        &format!("remote interface method ordinal {ordinal} signature"),
                    )?;
                }
            }
        }
    }
    for target in candidate.synthetic_callbacks() {
        prove_callable_signature(
            facts,
            target.signature(),
            VerificationLocation::Table {
                table: CandidateTable::SyntheticCallbacks,
                row: target.index().get(),
            },
            "synthetic callback signature",
        )?;
    }
    for target in candidate.host_effect_adapters() {
        prove_native_signature(
            facts,
            target.signature(),
            VerificationLocation::Table {
                table: CandidateTable::HostEffectAdapters,
                row: target.index().get(),
            },
            "host effect adapter signature",
        )?;
    }
    for target in candidate.intrinsics() {
        prove_native_signature(
            facts,
            target.signature(),
            VerificationLocation::Table {
                table: CandidateTable::Intrinsics,
                row: target.index().get(),
            },
            "intrinsic signature",
        )?;
    }
    Ok(())
}

fn prove_callable_signature(
    facts: &ConcreteValueFacts,
    signature: &LinkedCallableSignature,
    location: VerificationLocation,
    owner: &str,
) -> Result<(), VerificationError> {
    prove_signature_parts(
        facts,
        SignatureParts {
            parameter_types: signature.parameter_types(),
            parameter_plans: signature.parameter_plans(),
            result_types: signature.result_types(),
            result_plans: signature.result_plans(),
        },
        location,
        owner,
    )
}

fn prove_native_signature(
    facts: &ConcreteValueFacts,
    signature: &LinkedNativeCallableSignature,
    location: VerificationLocation,
    owner: &str,
) -> Result<(), VerificationError> {
    prove_signature_parts(
        facts,
        SignatureParts {
            parameter_types: signature.parameter_types(),
            parameter_plans: signature.parameter_plans(),
            result_types: signature.result_types(),
            result_plans: signature.result_plans(),
        },
        location,
        owner,
    )
}

struct SignatureParts<'a> {
    parameter_types: &'a [TypeIndex],
    parameter_plans: &'a [LinkedValueTransferPlan],
    result_types: &'a [TypeIndex],
    result_plans: &'a [LinkedValueTransferPlan],
}

fn prove_signature_parts(
    facts: &ConcreteValueFacts,
    signature: SignatureParts<'_>,
    location: VerificationLocation,
    owner: &str,
) -> Result<(), VerificationError> {
    for (ordinal, (ty, plan)) in signature
        .parameter_types
        .iter()
        .copied()
        .zip(signature.parameter_plans)
        .enumerate()
    {
        prove_position(
            facts,
            ty,
            plan,
            location,
            format!("{owner} parameter ordinal {ordinal}"),
        )?;
    }
    for (ordinal, (ty, plan)) in signature
        .result_types
        .iter()
        .copied()
        .zip(signature.result_plans)
        .enumerate()
    {
        prove_position(
            facts,
            ty,
            plan,
            location,
            format!("{owner} result ordinal {ordinal}"),
        )?;
    }
    Ok(())
}
