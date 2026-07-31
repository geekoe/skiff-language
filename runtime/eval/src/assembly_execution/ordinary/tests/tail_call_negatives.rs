use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_compiler::CompilerPlatformSources;
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, HydratedPackageCode, LinkedCallTarget, LinkedExprIr,
    PublicationResourceTable,
};
use skiff_runtime_model::runtime_value::{ActorRef, RuntimeValue};
use skiff_test_runner::canonical_package::compile_package_project;

use super::{
    service_error_consumer::{ConsumerTopology, ProviderFailureKind, ServiceErrorConsumerFixture},
    *,
};
use crate::error::RuntimeError;

const DEPTH_LIMIT_MINUS_ONE: usize = 31;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct CanonicalNegativeFixture {
    image: Arc<AssemblyExecutionImage>,
    eval_target: RuntimeAssemblyEvalTarget,
    entry_addr: ExecutableAddr,
}

impl CanonicalNegativeFixture {
    fn from_files(mut files: Vec<FileIrUnit>, entry_file: usize, entry_executable: usize) -> Self {
        for file in &mut files {
            skiff_artifact_identity::assign_file_ir_identity(file)
                .expect("negative File IR should receive a canonical identity");
        }

        let mut package = private_package("example.tail-call-negative-matrix", &files[0]);
        package.files = files.iter().map(file_ref).collect();
        skiff_artifact_identity::assign_package_artifact_identities(&mut package)
            .expect("negative package should receive canonical identities");
        let package_ref = package_ref(&package);
        let assembly = runtime_assembly(package_ref.clone(), "assembly:tail-call-negatives");
        let image =
            crate::test_support::link_package_fixture(assembly.clone(), vec![(package, files)]);
        Self::from_image(
            image,
            assembly.assembly_identity,
            package_ref.package_build_id,
            ExecutableAddr::package(0, entry_file, entry_executable),
        )
    }

    fn from_hydrated_package(
        package: PackageArtifact,
        files: Vec<FileIrUnit>,
        schema_index: PackageSchemaIndex,
        entry_file: usize,
        entry_executable: usize,
    ) -> Self {
        let package_ref = skiff_artifact_identity::package_artifact_ref(&package)
            .expect("compiled package should have canonical identities");
        let assembly = runtime_assembly(package_ref.clone(), "assembly:tail-call-negative-actor");
        let image = skiff_runtime_linker::link_package_fixture_from_runtime_assembly(
            &assembly,
            [HydratedPackageCode::new(
                Arc::new(package),
                files.into_iter().map(Arc::new).collect(),
                PublicationResourceTable::default(),
            )
            .with_schema_index(Arc::new(schema_index))],
        )
        .expect("compiled Actor package should form an execution image");
        Self::from_image(
            image,
            assembly.assembly_identity,
            package_ref.package_build_id,
            ExecutableAddr::package(0, entry_file, entry_executable),
        )
    }

    fn from_image(
        image: Arc<AssemblyExecutionImage>,
        assembly_identity: AssemblyIdentity,
        package_build_id: PackageBuildId,
        entry_addr: ExecutableAddr,
    ) -> Self {
        let activation = activation_context(assembly_identity, package_build_id);
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
            activation: Arc::clone(&activation),
        });
        let request = RequestActivationContext::begin(activation)
            .expect("negative request generation should begin");
        let eval_target = RuntimeAssemblyEvalTarget::new(Arc::clone(&image), request, resolver)
            .expect("negative image and activation should form an eval target");
        Self {
            image,
            eval_target,
            entry_addr,
        }
    }

    async fn execute(
        self,
        args: Vec<RuntimeValue>,
        initial_depth: usize,
    ) -> Result<RuntimeValue, RuntimeError> {
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let context = execution_context(&interpreter, self.eval_target)
            .with_program_call_depth_for_test(initial_depth);
        interpreter
            .execute_runtime_assembly_addr(
                context,
                &mut RequestHeap::default(),
                &self.entry_addr,
                args,
            )
            .await
    }

    fn expression(&self, expression: usize) -> &LinkedExprIr {
        &self.image.execution_packages()[0].files()[0].executables[self.entry_addr.executable]
            .body
            .expressions[expression]
    }
}

#[tokio::test]
async fn assembly_tail_call_negative_nested_call_argument_keeps_ordinary_depth_frame() {
    let mut file = FileIrUnit::empty("tail.negative.argument", "source:negative-argument");
    file.executables.push(nested_argument_caller());
    file.executables.push(identity_executable());
    file.executables
        .push(number_terminal("tail.negative.argument.inner", 7));
    let fixture = CanonicalNegativeFixture::from_files(vec![file], 0, 0);

    let LinkedExprIr::Call { call: outer } = fixture.expression(1) else {
        panic!("Return.value must remain the outer exact call");
    };
    assert!(matches!(outer.target, LinkedCallTarget::Executable { .. }));
    assert_eq!(outer.args.len(), 1);
    assert!(matches!(
        fixture.expression(outer.args[0].expression as usize),
        LinkedExprIr::Call {
            call: skiff_runtime_linked_program::CallIr {
                target: LinkedCallTarget::Executable { .. },
                ..
            }
        }
    ));

    let error = fixture
        .execute(Vec::new(), DEPTH_LIMIT_MINUS_ONE)
        .await
        .expect_err("a call evaluated as a tail-call argument must retain its ordinary frame");
    assert_program_depth_error(error);
}

#[tokio::test]
async fn assembly_tail_call_negative_catch_try_call_keeps_ordinary_depth_frame() {
    let mut file = FileIrUnit::empty("tail.negative.catch", "source:negative-catch");
    file.type_table.push(TypeDeclIr {
        name: "NeverThrown".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.executables.push(catch_caller());
    file.executables
        .push(number_terminal("tail.negative.catch.inner", 11));
    let fixture = CanonicalNegativeFixture::from_files(vec![file], 0, 0);

    let LinkedExprIr::Catch { try_expression, .. } = fixture.expression(2) else {
        panic!("Return.value must remain a catch wrapper");
    };
    assert!(matches!(
        fixture.expression(try_expression.expression as usize),
        LinkedExprIr::Call {
            call: skiff_runtime_linked_program::CallIr {
                target: LinkedCallTarget::Executable { .. },
                ..
            }
        }
    ));

    let error = fixture
        .execute(Vec::new(), DEPTH_LIMIT_MINUS_ONE)
        .await
        .expect_err("a call inside catch must retain its ordinary frame");
    assert_program_depth_error(error);
}

#[tokio::test]
async fn assembly_tail_call_negative_service_target_keeps_real_boundary_dispatch() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let target = fixture.caller_eval_target();
    let linked_caller =
        &target.execution_projection().image().execution_packages()[0].files()[0].executables[0];
    assert!(matches!(
        linked_caller.body.expressions.first(),
        Some(LinkedExprIr::Call {
            call: skiff_runtime_linked_program::CallIr {
                target: LinkedCallTarget::ActivationRelativeService { .. },
                ..
            }
        })
    ));

    let context = fixture.execution_context(&interpreter, target);
    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut RequestHeap::default(),
            fixture.caller_addr(),
            Vec::new(),
        )
        .await
        .expect_err("the real provider throw must return through the service boundary");
    assert!(
        matches!(error, RuntimeError::UserException(_)),
        "service dispatch must preserve its structured boundary exception, got {error:?}"
    );
    assert_no_internal_control_escape(&error);
}

#[tokio::test]
async fn assembly_tail_call_negative_actor_target_keeps_real_actor_dispatch() {
    let project_dir = TestDir::new("tail-call-negative-actor");
    project_dir.write(
        "package.yml",
        "id: example.com/tail-call-negative-actor\nversion: 1.0.0\n",
    );
    project_dir.write("api.yml", "{}\n");
    project_dir.write(
        "main.skiff",
        r#"
type UserActor {
  id: string,
  displayName: string,
}

actor UserActor {
  key(id)
  create(displayName: string)
}

impl UserActor {
  function create(self: UserActor, displayName: string) -> void {
    self.displayName = displayName
  }

  function rename(self: UserActor, value: string) -> string {
    self.displayName = value
    return self.displayName
  }
}

function invoke(actor: UserActor) -> string {
  return actor.rename("Grace")
}
"#,
    );

    let artifact_root = project_dir.path().join("artifacts");
    CanonicalArtifactStore::create(&artifact_root).expect("isolated canonical artifact store");
    let project = compile_package_project(
        &repository_platform_sources(),
        project_dir.path(),
        &artifact_root,
    )
    .expect("real Actor source should compile");
    let files = project
        .package
        .file_ir_units
        .iter()
        .map(|published| published.unit.clone())
        .collect::<Vec<_>>();
    let (entry_file, entry_executable) = files
        .iter()
        .enumerate()
        .find_map(|(file_index, file)| {
            file.executables
                .iter()
                .position(|executable| executable.symbol.ends_with(".invoke"))
                .map(|executable_index| (file_index, executable_index))
        })
        .expect("compiled Actor source should contain invoke");
    let fixture = CanonicalNegativeFixture::from_hydrated_package(
        project.package.artifact.clone(),
        files,
        project.package.package_schema_index.clone(),
        entry_file,
        entry_executable,
    );

    assert!(
        fixture.image.execution_packages()[0].files()[entry_file].executables[entry_executable]
            .body
            .expressions
            .iter()
            .any(|expression| matches!(
                expression,
                LinkedExprIr::Call {
                    call: skiff_runtime_linked_program::CallIr {
                        target: LinkedCallTarget::ActorDispatch { .. },
                        ..
                    }
                }
            ))
    );
    let error = fixture
        .execute(
            vec![RuntimeValue::ActorRef(ActorRef::new(
                "example.com/tail-call-negative-actor",
                "main.UserActor",
                "builtin:string",
                "actor-id-v1",
                br#""negative-actor""#.to_vec(),
                "sha256:negative-actor",
                Some(7),
            ))],
            0,
        )
        .await
        .expect_err("the test Actor capability must receive the real Actor dispatch");
    assert!(
        error.ordinary_payload().is_some_and(|payload| payload
            .message
            .contains("test actor capability is unavailable")),
        "Actor dispatch must surface the real capability result, got {error:?}"
    );
    assert_no_internal_control_escape(&error);
}

#[tokio::test]
async fn assembly_tail_call_negative_native_target_keeps_real_native_dispatch() {
    let target = NativeTarget {
        namespace: "std.string".to_string(),
        symbol: "isAsciiDigits".to_string(),
        binding_key: Some("std.string.isAsciiDigits".to_string()),
        metadata: BTreeMap::new(),
    };
    let mut file = FileIrUnit::empty("tail.negative.native", "source:negative-native");
    file.external_refs.native_targets.push(target.clone());
    file.executables.push(direct_target_caller(
        "tail.negative.native",
        TypeRefIr::builtin("bool"),
        vec![string_expression("12345")],
        CallTargetIr::Native { target },
    ));
    let fixture = CanonicalNegativeFixture::from_files(vec![file], 0, 0);
    assert!(matches!(
        fixture.expression(1),
        LinkedExprIr::Call {
            call: skiff_runtime_linked_program::CallIr {
                target: LinkedCallTarget::Native { .. },
                ..
            }
        }
    ));

    let value = fixture
        .execute(Vec::new(), DEPTH_LIMIT_MINUS_ONE)
        .await
        .expect("native target must use its real ordinary dispatcher");
    assert_eq!(value, RuntimeValue::Bool(true));
}

#[tokio::test]
async fn assembly_tail_call_negative_builtin_target_keeps_real_builtin_dispatch() {
    let mut file = FileIrUnit::empty("tail.negative.builtin", "source:negative-builtin");
    file.executables.push(direct_target_caller(
        "tail.negative.builtin",
        TypeRefIr::builtin("number"),
        vec![string_expression("tail")],
        CallTargetIr::Builtin {
            op: "string.length".to_string(),
        },
    ));
    let fixture = CanonicalNegativeFixture::from_files(vec![file], 0, 0);
    assert!(matches!(
        fixture.expression(1),
        LinkedExprIr::Call {
            call: skiff_runtime_linked_program::CallIr {
                target: LinkedCallTarget::Builtin { .. },
                ..
            }
        }
    ));

    let value = fixture
        .execute(Vec::new(), DEPTH_LIMIT_MINUS_ONE)
        .await
        .expect("builtin target must use its real ordinary dispatcher");
    assert_eq!(value, RuntimeValue::Number(4.0));
}

fn runtime_assembly(package: PackageArtifactRef, identity: &str) -> RuntimeAssembly {
    RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new(identity),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot { package }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    }
}

fn nested_argument_caller() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.negative.argument.caller".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("number"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: direct_return_body(vec![
            call_expression(
                CallTargetIr::LocalExecutable {
                    executable_index: 2,
                },
                Vec::new(),
            ),
            call_expression(
                CallTargetIr::LocalExecutable {
                    executable_index: 1,
                },
                vec![ExprRefIr { expression: 0 }],
            ),
        ]),
        source_span: None,
    }
}

fn identity_executable() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.negative.argument.identity".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: TypeRefIr::builtin("number"),
        }],
        return_type: TypeRefIr::builtin("number"),
        self_type: None,
        slots: SlotLayout {
            slots: vec![SlotIr {
                index: 0,
                name: "value".to_string(),
                kind: SlotKind::Param,
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: direct_return_body(vec![ExprIr::LoadSlot { slot: 0 }]),
        source_span: None,
    }
}

fn catch_caller() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.negative.catch.caller".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("Json"),
        self_type: None,
        slots: SlotLayout {
            slots: vec![SlotIr {
                index: 0,
                name: "$caught".to_string(),
                kind: SlotKind::Temp,
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: direct_return_body(vec![
            call_expression(
                CallTargetIr::LocalExecutable {
                    executable_index: 1,
                },
                Vec::new(),
            ),
            ExprIr::LoadSlot { slot: 0 },
            ExprIr::Catch {
                try_expression: ExprRefIr { expression: 0 },
                catch_slot: 0,
                catch_type: TypeRefIr::LocalType { type_index: 0 },
                body: ExprRefIr { expression: 1 },
            },
        ]),
        source_span: None,
    }
}

fn number_terminal(symbol: &str, value: i64) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("number"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: direct_return_body(vec![ExprIr::Literal {
            value: LiteralIr::Number {
                value: serde_json::Number::from(value),
            },
        }]),
        source_span: None,
    }
}

fn direct_target_caller(
    symbol: &str,
    return_type: TypeRefIr,
    mut arguments: Vec<ExprIr>,
    target: CallTargetIr,
) -> ExecutableIr {
    let args = (0..arguments.len())
        .map(|expression| ExprRefIr {
            expression: expression as u32,
        })
        .collect();
    arguments.push(call_expression(target, args));
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type,
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: direct_return_body(arguments),
        source_span: None,
    }
}

fn direct_return_body(expressions: Vec<ExprIr>) -> ExecutableBody {
    ExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }],
        }],
        statements: vec![StmtIr::Return {
            value: Some(ExprRefIr {
                expression: (expressions.len() - 1) as u32,
            }),
        }],
        expressions,
    }
}

fn call_expression(target: CallTargetIr, args: Vec<ExprRefIr>) -> ExprIr {
    ExprIr::Call {
        call: CallIr {
            target,
            site: test_instruction_site(),
            args,
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    }
}

fn string_expression(value: &str) -> ExprIr {
    ExprIr::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

fn assert_program_depth_error(error: RuntimeError) {
    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded {
            ref resource,
            limit: 32,
            current: 32,
            requested_delta: 1,
            ..
        } if resource == "programCallDepth"
    ));
}

fn assert_no_internal_control_escape(error: &RuntimeError) {
    assert!(
        !format!("{error:?}").contains("leaked an internal tail-call frame"),
        "excluded target leaked evaluator-internal control: {error:?}"
    );
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval must live below the Skiff root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("repository platform sources")
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-runtime-eval-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative_path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        let path = self.path.join(relative_path);
        let parent = path.parent().expect("fixture file parent");
        fs::create_dir_all(parent).expect("fixture parent directory");
        fs::write(path, contents).expect("fixture file");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
