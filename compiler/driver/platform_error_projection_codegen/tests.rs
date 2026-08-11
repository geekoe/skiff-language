use std::{fs, path::PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::resolve_platform_error_projection_catalog;

#[path = "../../../artifact-model/src/platform_error_projection/generated.rs"]
mod generated_artifact_model;

use super::{
    check_outputs,
    fingerprint::{entry_preimage, fingerprint_catalog, registry_preimage_for_catalog},
    generate_outputs, render_all, stage_and_publish_with, PreparedOutput, RenderedOutput,
    ARTIFACT_MODEL_OUTPUT, GENERATED_HEADER, REQUEST_CONTRACT_OUTPUT,
};

const EXPECTED_ENTRIES: [(&str, &str); 21] = [
    (
        "config.DecodeError",
        "sha256:ffa1b591013fa33f49d8c3e36fd1f90eac8611ccd6ea6a816d251ef8e1a990f2",
    ),
    (
        "std.actor.ActivationTimeoutError",
        "sha256:c9036d5d14f9ecd9dc41fbe574837ba7dd836f4ed3c79771eb28ab0235d5187d",
    ),
    (
        "std.actor.MethodInvocationTimeoutError",
        "sha256:3364033b21bc82045bbd7a71bfc3275a4cf30e8a5934c43e9c2ea0f4cc162ccc",
    ),
    (
        "std.bytes.DecodeError",
        "sha256:49b4ec80eb89f71a682e0b8c53a923dfbb3b4da85333775030c8bd0b1271de48",
    ),
    (
        "std.collection.ArrayIndexOutOfBoundsError",
        "sha256:00d01d71fb87454846f086cef35f5a3f0b5bef0670000daa15254fd0fe7c24cd",
    ),
    (
        "std.collection.JsonObjectPropertyNotFoundError",
        "sha256:71e1d94e5c10440107e0d17e58a1eef56b2485b53a1b0139245d3b5f941abc1b",
    ),
    (
        "std.collection.MapKeyNotFoundError",
        "sha256:1abcd53ea4b33e421fdd4dccfbc24c4d9b9c749ba3b6ec9abe9bebe85e431ede",
    ),
    (
        "std.db.ConflictError",
        "sha256:4511f1de74c636bdc0ad8049b4467ab1559754b6cb088ce768ca05f3d669aebf",
    ),
    (
        "std.db.ConstraintError",
        "sha256:e60e20b4b08ee99594887ece6061e36959f8f78f0ad9f27dd31bf335133038cf",
    ),
    (
        "std.db.DecodeError",
        "sha256:ac8985db6ff4101c42a6bb671067611c0e48e241ca356837d46425fba1d6dfa8",
    ),
    (
        "std.error.InstructionLimitExceededError",
        "sha256:c0e31ea825712c96a4506771d251d91ea7d482fd3284773723b7bff66063cd88",
    ),
    (
        "std.error.TimeoutError",
        "sha256:caa5a805c9f1766bb4c04c5f5c27add7ddfc956f82fc20f421f0e19cf96f68f6",
    ),
    (
        "std.file.FileError",
        "sha256:7e325895d2caadb6d6b27be8cf3b36c30c025c11e6ab097316118488f2128b77",
    ),
    (
        "std.http.HttpError",
        "sha256:1482750c71d3022bb47d8d594b2e5aeb1e177b2a5a1caf41125ef80709b7c141",
    ),
    (
        "std.http.RequestTimeoutError",
        "sha256:6fe26120b3ce3d94515e4b22ee4047765f705294a4bee4c55072a8db3049022f",
    ),
    (
        "std.json.DecodeError",
        "sha256:f0bf4ad3840906cc17b21453da98d49ad745f181180fb5269eca21d7be222c75",
    ),
    (
        "std.number.DecodeError",
        "sha256:96d0818e3b2239613a8f7c068dad6e9f7ec30684b7ac96c670a956a0d0908cfc",
    ),
    (
        "std.service.ProtocolError",
        "sha256:b4634d3924d59291be0c0f25b2806c2789bbc3093a5c8bbb7dead558cf64dc1a",
    ),
    (
        "std.service.ProviderUnavailableError",
        "sha256:875f4d82fb3306eead3ef84baa40a0cc82a65237d30d6dcc4f1e32ad0dd7166e",
    ),
    (
        "std.time.DecodeError",
        "sha256:203bb25750e7740f404540d31a4c86c9de8b1177e6a71b4249f95a5a50130a61",
    ),
    (
        "std.websocket.WebSocketRequestError",
        "sha256:72c5df02a2e86f1c45aeac73188e7236b488dbebe29669677bbb92b9bd2ec071",
    ),
];

const EXPECTED_REGISTRY_FINGERPRINT: &str =
    "sha256:2c9999fba4365d136499ed6c234f0e26f091ec0a8b12e160ba2eed2f2d2ad1ea";

#[test]
fn repository_catalog_is_exact_sorted_all_and_only_and_unversioned() {
    let catalog = resolved_repository_catalog();
    let actual = catalog
        .entries()
        .iter()
        .map(|entry| entry.projection_key())
        .collect::<Vec<_>>();
    let expected = EXPECTED_ENTRIES
        .iter()
        .map(|(key, _)| *key)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 21);
    assert!(actual.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(actual.iter().all(|key| !has_numeric_version_segment(key)));
}

#[test]
fn exact_entry_and_whole_registry_fingerprints_are_frozen() {
    let catalog = resolved_repository_catalog();
    let fingerprinted = fingerprint_catalog(&catalog).unwrap();
    let actual = fingerprinted
        .entries
        .iter()
        .map(|entry| (entry.resolved.projection_key(), entry.fingerprint.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(actual.as_slice(), EXPECTED_ENTRIES.as_slice());
    assert_eq!(
        fingerprinted.registry_fingerprint,
        EXPECTED_REGISTRY_FINGERPRINT
    );
    assert_eq!(
        std::str::from_utf8(&registry_preimage_for_catalog(&fingerprinted).unwrap()).unwrap(),
        expected_registry_preimage()
    );
    for entry in &fingerprinted.entries {
        let preimage_bytes = entry_preimage(entry.resolved).unwrap();
        let preimage = std::str::from_utf8(&preimage_bytes).unwrap();
        assert_eq!(
            preimage,
            expected_entry_preimage(entry.resolved.projection_key()),
            "{} preimage drifted",
            entry.resolved.projection_key()
        );
    }
}

#[test]
fn schema_and_policy_mutations_keep_key_but_change_entry_and_registry_fingerprints() {
    let catalog = resolved_repository_catalog();
    let fingerprinted = fingerprint_catalog(&catalog).unwrap();
    let original_entry = &fingerprinted.entries[0];
    let original_preimage = entry_preimage(original_entry.resolved).unwrap();
    let original_value: Value = serde_json::from_slice(&original_preimage).unwrap();
    for (field, replacement) in [
        (
            "schema",
            Value::String("skiff-platform-error-projection-entry-v2".to_owned()),
        ),
        (
            "publicMessagePolicy",
            Value::String("changedPolicy".to_owned()),
        ),
    ] {
        let mut mutated = original_value.clone();
        mutated[field] = replacement;
        assert_eq!(
            mutated["projectionKey"], original_value["projectionKey"],
            "mutation must retain the canonical public symbol key"
        );
        let mutated_bytes = skiff_canonical_json::canonical_json_bytes(&mutated).unwrap();
        let mutated_fingerprint = sha256(&mutated_bytes);
        assert_ne!(mutated_fingerprint, original_entry.fingerprint);

        let registry_preimage = registry_preimage_for_catalog(&fingerprinted).unwrap();
        let mut registry: Value = serde_json::from_slice(&registry_preimage).unwrap();
        registry["entries"][0]["entryFingerprint"] = Value::String(mutated_fingerprint);
        let registry_bytes = skiff_canonical_json::canonical_json_bytes(&registry).unwrap();
        assert_ne!(sha256(&registry_bytes), fingerprinted.registry_fingerprint);
    }
}

#[test]
fn render_is_deterministic_and_covers_current_typed_dto_shapes() {
    let catalog = resolved_repository_catalog();
    let first = render_all(&catalog).unwrap();
    let second = render_all(&catalog).unwrap();
    assert_eq!(
        first
            .iter()
            .map(|output| (&output.relative_path, &output.bytes))
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|output| (&output.relative_path, &output.bytes))
            .collect::<Vec<_>>()
    );
    let artifact = rendered_text(&first, ARTIFACT_MODEL_OUTPUT);
    let request = rendered_text(&first, REQUEST_CONTRACT_OUTPUT);
    assert_eq!(artifact.lines().next(), Some(GENERATED_HEADER));
    assert_eq!(request.lines().next(), Some(GENERATED_HEADER));
    assert!(artifact.contains("pub const ALL: [Self; 21]"));
    assert!(artifact.contains(
        "// Canonical public error symbols end in Error; path-derived Rust variants preserve that suffix."
    ));
    assert!(artifact.contains("#[allow(clippy::enum_variant_names)]"));
    assert!(
        artifact.contains("PlatformErrorProjectionKey::parse_strict(descriptor.key().as_str())")
    );
    assert!(artifact.contains("platform_error_projection_descriptor(parsed)"));
    assert!(
        artifact.contains("platform_error_projection_descriptor_by_key(descriptor.key().as_str())")
    );
    assert!(artifact.contains("impl<'de> Deserialize<'de> for PlatformErrorProjectionRegistryRef"));
    assert!(artifact.contains("pub fn registry_id(&self) -> &str"));
    assert!(artifact.contains("pub const fn registry_version(&self) -> u32"));
    assert!(artifact.contains("pub fn fingerprint(&self) -> &str"));
    assert!(artifact.contains("current_platform_error_projection_registry_ref"));
    assert!(artifact.contains(
        "pub fn platform_error_projection_registry() -> &'static [PlatformErrorProjectionDescriptor]"
    ));
    assert!(!artifact.contains("pub static PLATFORM_ERROR_PROJECTION_REGISTRY"));
    for getter in [
        "key",
        "nominal_identity",
        "entry_fingerprint",
        "codec_version",
        "producer_family",
        "semantic_adapter_owner",
        "public_message_policy",
        "envelope_kind",
        "fallback_policy",
    ] {
        assert!(artifact.contains(&format!("pub const fn {getter}(&self)")));
    }
    for field in [
        "key",
        "nominal_identity",
        "entry_fingerprint",
        "codec_version",
        "producer_family",
        "semantic_adapter_owner",
        "public_message_policy",
        "envelope_kind",
        "fallback_policy",
    ] {
        assert!(!artifact.contains(&format!("pub {field}:")));
    }
    assert!(!artifact.contains("pub registry_id: String"));
    assert!(!artifact.contains("pub registry_version: u32"));
    assert!(!artifact.contains("pub fingerprint: String"));
    assert!(!artifact.contains("pub fn current() -> Self"));
    let deserialize_impl = artifact
        .split_once("impl<'de> Deserialize<'de> for PlatformErrorProjectionRegistryRef")
        .unwrap()
        .1
        .split_once("pub fn current_platform_error_projection_registry_ref")
        .unwrap()
        .0;
    let construct = deserialize_impl.find("let descriptor = Self").unwrap();
    let validate = deserialize_impl
        .find("validate_platform_error_projection_registry_ref_shape(&descriptor)")
        .unwrap();
    let accept = deserialize_impl.find("Ok(descriptor)").unwrap();
    assert!(construct < validate);
    assert!(validate < accept);
    for (key, fingerprint) in EXPECTED_ENTRIES {
        assert!(artifact.contains(&format!("#[serde(rename = {key:?})]")));
        assert!(artifact.contains(fingerprint));
    }
    assert!(request.contains("pub struct StdCollectionMapKeyNotFoundErrorPayload {}"));
    assert!(request.contains("deserialize_with = \"deserialize_required_nullable\""));
    assert!(request.contains("#[serde(tag = \"kind\", deny_unknown_fields)]"));
    for branch in [
        "connectionUnavailable",
        "transportUnavailable",
        "protocolError",
        "resourceLimit",
        "remote",
    ] {
        assert!(request.contains(&format!("#[serde(rename = {branch:?})]")));
    }
    let websocket_union = request
        .split_once("pub enum StdWebsocketWebSocketRequestErrorPayload {\n")
        .unwrap()
        .1
        .split_once("\n}\n\n")
        .unwrap()
        .0;
    assert!(!websocket_union.contains("pub "));
    for variant_field in [
        "message: String,",
        "code: i64,",
        "data: Option<serde_json::Value>,",
    ] {
        assert!(websocket_union.contains(variant_field));
    }
    let nullable_record = request
        .split_once("pub struct StdHttpHttpErrorPayload {\n")
        .unwrap()
        .1
        .split_once("\n}\n\n")
        .unwrap()
        .0;
    assert!(nullable_record.contains("pub detail: Option<serde_json::Value>"));
    assert!(nullable_record.contains("pub message: String"));
    assert!(request.contains("canonical != raw_payload"));
    assert!(request.contains("UnknownValid"));
    assert!(request.contains("skiff_canonical_json::canonical_json_bytes(value)"));
    assert!(!request.contains("fn canonical_json_value"));
    assert!(!request.contains("pub fn materialize_platform_error_projection_payload"));
    assert!(!request.contains("pub entry_fingerprint: &'static str"));
    assert!(request.contains("descriptor.entry_fingerprint()"));
    assert!(!request.contains("descriptor.entry_fingerprint,"));
}

#[test]
fn same_key_different_fingerprint_is_classified_before_typed_materialization() {
    let catalog = resolved_repository_catalog();
    let rendered = render_all(&catalog).unwrap();
    let request = rendered_text(&rendered, REQUEST_CONTRACT_OUTPUT);
    let decode = request
        .split_once("pub fn decode_platform_error_projection_payload(")
        .unwrap()
        .1
        .split_once("fn materialize_platform_error_projection_payload(")
        .unwrap()
        .0;
    let exact_pair_gate = decode
        .find("if entry_fingerprint != descriptor.entry_fingerprint()")
        .unwrap();
    let unknown_outcome = decode[exact_pair_gate..]
        .find("return Ok(PlatformErrorProjectionDecodeOutcome::UnknownValid)")
        .unwrap()
        + exact_pair_gate;
    let typed_call = decode
        .find("materialize_platform_error_projection_payload(key, raw_payload)")
        .unwrap();
    assert!(exact_pair_gate < unknown_outcome);
    assert!(unknown_outcome < typed_call);
}

#[test]
fn generated_output_check_detects_missing_and_drift_without_writing() {
    let fixture = OutputFixture::new("check");
    let rendered = fixture_outputs("first", "second");
    generate_outputs(&fixture.root, fixture_outputs("first", "second")).unwrap();
    check_outputs(&fixture.root, &rendered).unwrap();

    let first = fixture.root.join(ARTIFACT_MODEL_OUTPUT);
    let second = fixture.root.join(REQUEST_CONTRACT_OUTPUT);
    fs::write(&first, "drift").unwrap();
    let error = check_outputs(&fixture.root, &rendered).unwrap_err();
    assert!(error.to_string().contains("has drifted"), "{error}");
    assert_eq!(fs::read_to_string(&first).unwrap(), "drift");
    assert_eq!(fs::read_to_string(&second).unwrap(), generated("second"));

    fs::write(&first, generated("first")).unwrap();
    fs::remove_file(&second).unwrap();
    let error = check_outputs(&fixture.root, &rendered).unwrap_err();
    assert!(error.to_string().contains("is missing"), "{error}");
    assert_eq!(fs::read_to_string(&first).unwrap(), generated("first"));
    assert!(!second.exists());
}

#[test]
fn generate_stages_all_outputs_before_publication_and_leaves_no_partial_file() {
    let fixture = OutputFixture::new("atomic");
    fs::create_dir_all(
        fixture
            .root
            .join("artifact-model/src/platform_error_projection"),
    )
    .unwrap();
    fs::create_dir_all(
        fixture
            .root
            .join("runtime/request-contract/src/platform_error_projection/generated.rs"),
    )
    .unwrap();
    let first = fixture.root.join(ARTIFACT_MODEL_OUTPUT);
    fs::write(&first, "original").unwrap();

    let error =
        generate_outputs(&fixture.root, fixture_outputs("replacement", "blocked")).unwrap_err();
    assert!(error.to_string().contains("not a regular file"), "{error}");
    assert_eq!(fs::read_to_string(&first).unwrap(), "original");
    assert!(temporary_siblings(&first).is_empty());
}

#[test]
fn publication_failure_rolls_back_an_already_replaced_output() {
    let fixture = OutputFixture::new("rollback");
    let first = fixture.root.join("artifact-model/src/first.rs");
    let second = fixture.root.join("runtime/request-contract/src/second.rs");
    fs::write(&first, "original-first").unwrap();
    fs::write(&second, "original-second").unwrap();
    let mut prepared = vec![
        prepared_output(first.clone(), b"new-first"),
        prepared_output(second.clone(), b"new-second"),
    ];
    let mut publication_index = 0;
    let error = stage_and_publish_with(&mut prepared, |staged, target| {
        let current = publication_index;
        publication_index += 1;
        if current == 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected second publication failure",
            ));
        }
        fs::rename(staged, target)
    })
    .unwrap_err();
    super::cleanup_staged_and_backups(&mut prepared).unwrap();

    assert!(
        error
            .to_string()
            .contains("injected second publication failure"),
        "{error}"
    );
    assert_eq!(fs::read_to_string(&first).unwrap(), "original-first");
    assert_eq!(fs::read_to_string(&second).unwrap(), "original-second");
    assert!(temporary_siblings(&first).is_empty());
    assert!(temporary_siblings(&second).is_empty());
}

fn resolved_repository_catalog() -> skiff_compiler_source::ResolvedPlatformErrorProjectionCatalog {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .unwrap();
    let platform_sources = CompilerPlatformSources::new(&root).unwrap();
    resolve_platform_error_projection_catalog(&platform_sources).unwrap()
}

fn rendered_text<'a>(outputs: &'a [RenderedOutput], path: &str) -> &'a str {
    let output = outputs
        .iter()
        .find(|output| output.relative_path == path)
        .unwrap();
    std::str::from_utf8(&output.bytes).unwrap()
}

fn has_numeric_version_segment(key: &str) -> bool {
    key.split('.').any(|segment| {
        segment.strip_prefix('v').is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn fixture_outputs(first: &str, second: &str) -> Vec<RenderedOutput> {
    vec![
        RenderedOutput {
            relative_path: ARTIFACT_MODEL_OUTPUT,
            bytes: generated(first).into_bytes(),
        },
        RenderedOutput {
            relative_path: REQUEST_CONTRACT_OUTPUT,
            bytes: generated(second).into_bytes(),
        },
    ]
}

fn generated(body: &str) -> String {
    format!("{GENERATED_HEADER}\n{body}\n")
}

fn prepared_output(target: PathBuf, bytes: &[u8]) -> PreparedOutput {
    PreparedOutput {
        target,
        bytes: bytes.to_vec(),
        staged: None,
        backup: None,
        existed: true,
        published: false,
    }
}

fn temporary_siblings(target: &std::path::Path) -> Vec<PathBuf> {
    let prefix = format!(
        ".{}.skiff-codegen-",
        target.file_name().unwrap().to_string_lossy()
    );
    fs::read_dir(target.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .collect()
}

struct OutputFixture {
    base: PathBuf,
    root: PathBuf,
}

impl OutputFixture {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "skiff-platform-error-codegen-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("repository");
        fs::create_dir_all(root.join("artifact-model/src")).unwrap();
        fs::create_dir_all(root.join("runtime/request-contract/src")).unwrap();
        let root = root.canonicalize().unwrap();
        Self { base, root }
    }
}

impl Drop for OutputFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn expected_registry_preimage() -> &'static str {
    include_str!("registry_preimage.golden.json")
        .strip_suffix('\n')
        .unwrap()
}

fn expected_entry_preimage(key: &str) -> &'static str {
    include_str!("entry_preimages.golden.txt")
        .lines()
        .find_map(|line| {
            let (line_key, preimage) = line.split_once('\t').unwrap();
            (line_key == key).then_some(preimage)
        })
        .unwrap_or_else(|| panic!("unexpected projection key {key}"))
}
