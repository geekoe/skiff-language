use std::collections::BTreeSet;

use skiff_artifact_model::{
    bytecode::{BytecodeFunctionOrigin, BytecodeRelocation, ValueDropPlan, ValueTransferPlan},
    LinkedOperandKind, Opcode, PackageExecutableCoordinate, ValueTransferPlanKind,
};
use skiff_runtime_bytecode_verifier::VerifiedResumeKind;

use super::fixture::{BuildOutcome, FixtureSpec, PublishedFixture};

const VCP_PATH: &str = "/phase-5/vcp";

pub fn published_positive(prefix: &str) -> PublishedFixture {
    match FixtureSpec::positive().build(prefix) {
        BuildOutcome::Published(fixture) => fixture,
        BuildOutcome::Rejected { error_chain, .. } => panic!(
            "the production source/publication carrier did not reach this stage: {error_chain}"
        ),
    }
}

#[test]
fn phase_5_stage_sentinel_admission_to_emission() {
    let fixture = published_positive("s2-emission");
    let bytecode = fixture.bytecode();
    let package = fixture.package_artifact();
    let deployment = fixture.deployment_artifact();
    let function = gateway_artifact_function(&package, &deployment, bytecode.view(), VCP_PATH);

    let host_bindings = function
        .instructions
        .iter()
        .filter(|instruction| instruction.descriptor.kind == Opcode::InvokeHost)
        .map(|instruction| {
            let relocation = function
                .relocations
                .get(instruction.operand(0) as usize)
                .expect("InvokeHost relocation index");
            let BytecodeRelocation::HostEffectRef(effect) = relocation else {
                panic!("InvokeHost must retain a typed HostEffectRef relocation")
            };
            effect
                .target
                .binding_key
                .as_deref()
                .expect("HostEffectRef exact canonical binding")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        host_bindings,
        [
            "std.http.client.request",
            "std.http.client.stream",
            "std.http.client.stream",
        ],
        "the exact VCP gateway must carry one request and two stream callsites"
    );

    let take_count = function
        .instructions
        .iter()
        .filter(|instruction| instruction.descriptor.kind.name() == "take_dense_field")
        .count();
    assert_eq!(
        take_count, 2,
        "left.body and right.body need two affine takes"
    );

    let stream_next_sites = function
        .instructions
        .iter()
        .filter(|instruction| instruction.descriptor.kind == Opcode::StreamNext)
        .collect::<Vec<_>>();
    assert_eq!(
        stream_next_sites.len(),
        2,
        "A and B need independent StreamNext sites"
    );
    for instruction in stream_next_sites {
        let resume_index = instruction.operand(1);
        let resume = bytecode
            .view()
            .resume_sites()
            .iter()
            .find(|resume| {
                resume.function_key == function.function_key
                    && resume.descriptor_index == resume_index
            })
            .expect("StreamNext carries an admitted resume descriptor");
        assert!(
            resume.end_resume_pc.is_some(),
            "StreamNext certifies natural end"
        );
    }

    let recursive_move_slots = function
        .frame_layout
        .slot_plans
        .iter()
        .filter(|plan| {
            matches!(
                plan,
                ValueTransferPlan::MoveOnly {
                    drop: ValueDropPlan::RecursiveShape { .. }
                }
            )
        })
        .count();
    assert_eq!(
        recursive_move_slots, 2,
        "the two HTTP stream handle locals require recursive remainder-drop plans"
    );
}

#[test]
fn phase_5_stage_sentinel_emission_to_link() {
    let fixture = published_positive("s3-link");
    let gateway = fixture.gateway(VCP_PATH);
    let image = fixture.link();
    let entry = image
        .http_gateway_entry(&gateway.ingress, &gateway.identity)
        .expect("production linker resolves the exact VCP gateway");
    let function = &image.functions()[entry.function().get() as usize];

    let target_indices = function
        .instructions()
        .iter()
        .filter(|instruction| instruction.opcode() == Opcode::InvokeHost)
        .map(|instruction| {
            let [resolved] = instruction.resolved_operands() else {
                panic!("InvokeHost must resolve exactly its typed host target operand")
            };
            assert_eq!(
                resolved.target().kind(),
                LinkedOperandKind::HostEffectAdapter,
                "InvokeHost resolved operand is not a host adapter index"
            );
            instruction.operands()[resolved.operand_ordinal() as usize]
        })
        .collect::<Vec<_>>();
    assert_eq!(
        target_indices.len(),
        3,
        "linked VCP has three exact host callsites"
    );
    assert_eq!(
        target_indices
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        2,
        "two stream calls share one exact typed target while request remains distinct"
    );

    let bindings = target_indices
        .iter()
        .map(|index| {
            image
                .host_effect_adapters()
                .get(*index as usize)
                .expect("host target index resolves in the production image")
                .binding_key()
                .as_str()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        bindings,
        [
            "std.http.client.request",
            "std.http.client.stream",
            "std.http.client.stream",
        ]
    );
}

#[test]
fn phase_5_stage_sentinel_link_to_verify() {
    let fixture = published_positive("s4-verify");
    let gateway = fixture.gateway(VCP_PATH);
    let image = fixture.link();
    let entry = image
        .http_gateway_entry(&gateway.ingress, &gateway.identity)
        .expect("verified image resolves exact VCP gateway");
    let function_index = entry.function();
    let function = &image.functions()[function_index.get() as usize];
    let sites = image
        .resume_sites()
        .rows()
        .iter()
        .filter(|site| site.function() == function_index)
        .collect::<Vec<_>>();
    assert_eq!(
        sites
            .iter()
            .filter(|site| matches!(site.kind(), VerifiedResumeKind::HostEffect))
            .count(),
        3,
        "verifier certifies every HTTP host call resume"
    );
    let stream_reads = sites
        .iter()
        .filter(|site| matches!(site.kind(), VerifiedResumeKind::StreamRead { .. }))
        .collect::<Vec<_>>();
    assert_eq!(stream_reads.len(), 2, "verifier certifies A/B item resumes");
    assert!(
        stream_reads.iter().all(|site| site.end_resume().is_some()),
        "verifier certifies the distinct natural-end continuation"
    );
    assert!(
        sites
            .iter()
            .any(|site| matches!(site.kind(), VerifiedResumeKind::StreamBackpressure)),
        "serverStream emit must retain an actual backpressure resume certificate"
    );

    assert_eq!(
        function
            .instructions()
            .iter()
            .filter(|instruction| instruction.opcode().name() == "take_dense_field")
            .count(),
        2,
        "verified code retains both affine body takes"
    );
    assert_eq!(
        function
            .frame()
            .slot_plans()
            .iter()
            .filter(|plan| plan.kind() == ValueTransferPlanKind::MoveOnly)
            .count(),
        2,
        "linked verifier input retains two recursive remainder-drop plans"
    );
}

fn gateway_artifact_function<'a>(
    package: &skiff_artifact_model::PackageArtifact,
    deployment: &skiff_artifact_model::ServiceDeployment,
    view: &'a skiff_artifact_model::StructurallyValidatedView,
    path: &str,
) -> &'a skiff_artifact_model::ValidatedFunction {
    let ingress = deployment
        .ingress
        .iter()
        .find(|binding| binding.selector.path == path)
        .expect("exact VCP ingress");
    let handler = deployment.gateway_entries[&ingress.gateway_entry_key]
        .handler
        .as_ref()
        .expect("VCP gateway handler");
    let target = &package.callable_links[handler].target;
    let coordinate = PackageExecutableCoordinate {
        file_ir_identity: target.file_ref.file_ir_identity.clone(),
        module_path: target.file_ref.module_path.clone(),
        executable_index: target.executable_index,
    };
    view.functions()
        .iter()
        .find(|function| {
            matches!(
                &function.origin,
                BytecodeFunctionOrigin::Executable { executable } if executable == &coordinate
            )
        })
        .expect("artifact contains the exact deployment gateway executable")
}
