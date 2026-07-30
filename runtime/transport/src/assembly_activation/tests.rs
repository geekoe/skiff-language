use serde::Deserialize;
use skiff_artifact_model::AssemblyActivationControl;

use super::{
    decode_assembly_activation_frame, encode_assembly_activation_frame,
    AssemblyActivationFrameDirection,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    assembly_activation_frames: Vec<GoldenFrame>,
    assembly_activation_mutations: Vec<Mutation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldenFrame {
    name: String,
    direction: String,
    control_index: usize,
    frame_hex: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mutation {
    name: String,
    direction: String,
    input: String,
}

#[test]
fn assembly_activation_frame_matches_shared_golden_bytes_and_directions() {
    let corpus = corpus();
    let controls: Vec<AssemblyActivationControl> = serde_json::from_str(include_str!(
        "../../../../cross-system-fixtures/package-service-ecosystem/control-wire.json"
    ))
    .expect("shared controls");
    assert_eq!(corpus.assembly_activation_frames.len(), controls.len());
    for golden in corpus.assembly_activation_frames {
        let direction = direction(&golden.direction);
        let control = &controls[golden.control_index];
        assert_eq!(control_type(control), golden.name);
        let expected = decode_hex(&golden.frame_hex);
        assert_eq!(
            encode_assembly_activation_frame(direction, control).unwrap(),
            expected,
            "{}",
            golden.name
        );
        assert_eq!(
            decode_assembly_activation_frame(direction, &expected).unwrap(),
            *control,
            "{}",
            golden.name
        );
        let reverse = opposite_direction(direction);
        assert!(
            encode_assembly_activation_frame(reverse, control).is_err(),
            "{} reverse encode direction",
            golden.name
        );
        assert!(
            decode_assembly_activation_frame(reverse, &expected).is_err(),
            "{} reverse decode direction",
            golden.name
        );
    }
}

#[test]
fn assembly_activation_frame_mutations_fail_closed() {
    for mutation in corpus().assembly_activation_mutations {
        assert!(
            decode_assembly_activation_frame(
                direction(&mutation.direction),
                &decode_hex(&mutation.input),
            )
            .is_err(),
            "{}",
            mutation.name
        );
    }
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../../cross-system-fixtures/package-service-ecosystem/runtime-wire.json"
    ))
    .expect("shared runtime wire corpus")
}

fn direction(value: &str) -> AssemblyActivationFrameDirection {
    match value {
        "routerToRuntime" => AssemblyActivationFrameDirection::RouterToRuntime,
        "runtimeToRouter" => AssemblyActivationFrameDirection::RuntimeToRouter,
        other => panic!("unknown direction {other}"),
    }
}

fn opposite_direction(
    direction: AssemblyActivationFrameDirection,
) -> AssemblyActivationFrameDirection {
    match direction {
        AssemblyActivationFrameDirection::RouterToRuntime => {
            AssemblyActivationFrameDirection::RuntimeToRouter
        }
        AssemblyActivationFrameDirection::RuntimeToRouter => {
            AssemblyActivationFrameDirection::RouterToRuntime
        }
    }
}

fn control_type(control: &AssemblyActivationControl) -> &'static str {
    match control {
        AssemblyActivationControl::Prepare { .. } => "prepare",
        AssemblyActivationControl::Prepared { .. } => "prepared",
        AssemblyActivationControl::Reject { .. } => "reject",
        AssemblyActivationControl::Commit { .. } => "commit",
        AssemblyActivationControl::Abort { .. } => "abort",
        AssemblyActivationControl::Register { .. } => "register",
    }
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    (0..value.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&value[offset..offset + 2], 16).unwrap())
        .collect()
}
