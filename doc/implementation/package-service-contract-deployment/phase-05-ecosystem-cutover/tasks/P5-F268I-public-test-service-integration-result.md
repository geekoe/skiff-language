# P5-F268I Public test service integration result

## 结果

完成。

F268 的三个非重复提交已经按指定顺序集成到
`codex/package-service-phase-05@63565894` 的 Phase 5 最终模型：

- `5ca8534` -> `8f7359eb`（`feat(testing): migrate public tests to test services`）；
- `15d9ab6` -> `2621dd57`（`feat(test): enforce typed effect sequences`）；
- `2350f1c` -> `82e5519e`（`fix(test): close public test service migration`）。

其中 `15d9ab6` 的内容在取入 `5ca8534` 并组合当前分支语义后已经
patch-equivalent；为保留输入顺序和审计链，`2621dd57` 是有意保留的空提交。集成编译闭合
提交为 `7787a21e`，任务授权更新为 `9ba210ff`。

最终代码同时保留了 F267 inline effect finalization、F271
`returnOrigins` / `directReturnOrigins`、F272 stable nested-field narrowing 和 F273
递归 alias canonical structured IR。未操作 stable，未访问外网，未 push。

## 冲突组合

### `5ca8534`

已确认的十个内容冲突逐项组合如下：

- `compiler/lowering/src/lowered.rs`：保留 F273 的
  `TypeResolutionModel` alias 展开输入，同时取得 F268 的 Package dependency ABI
  expectation map；最终调用以四个参数进入 publication-local ref 重写，并继续把失败归入
  File IR type finalization。
- `compiler/lowering/src/publication_local_refs.rs`：索引同时持有 F273 type resolution 和
  F268 dependency ABI expectation。重写时先递归展开 alias，再把当前 publication type
  或具备精确 ABI expectation 的 dependency Package symbol 规范化，没有退回字符串类型。
- `compiler/projection/src/package_artifact/callables/normalization.rs`：保留 F268
  implementation type/package symbol 规范化，并保留 F271 的结构化
  `ValueProjectionStep`、独立 `return_origins` / `direct_return_origins` 排序去重。
- `compiler/source/src/dependency_analysis.rs`：Package dependency facts 同时携带 F268 的
  精确 `PackageBuildId` 和 F271/F273 使用的 `PackageTypeRef`/schema facts；call target、
  constant 与类型投影共享同一精确 dependency identity。
- `compiler/source/src/type_resolution_model.rs`：按 access 区分 public symbol 与
  implementation symbol；同时构造 F268 `ArtifactSymbolicTypeIndex`
  （`by_symbol`/`by_slot`）和 F273 `PackageTypeSymbolIndex`。本地类型按 access 保持精确
  identity，artifact 类型仍区分 alias 与 nominal，并同时投影 canonical target/fields、
  Package-scoped textual projection 和 selected schema closure。
- `compiler/tests/std_package_imports.rs`：保留 `PackageSchema` 与 `PackageSymbol` 两组断言，
  并把 F271 的 `direct_return_origins` 精确断言加入 F268 std callable provenance：
  truncate 为 `[Fresh]`，HttpResponse constructor 为 `[Fresh]`。
- `runtime/eval/src/program_stream.rs`：保留 F267 的 target tombstone
  `contains_target` 行为；sequence 耗尽后失败关闭，不回落执行真实 stream producer。
- `test-runner/fixtures/package-service-host/consumer-tests/main.test.skiff`：保留 F267 最终
  typed unary/stream sequence 和 inline service effect body，同时迁移到 F268 普通
  `kind: test` service；被测 Package 通过 `import subject` 和 `subject/main` 访问，不恢复
  `root.*` dependency access。
- `test-runner/src/canonical_store.rs`：files/resources 写入完成后只插入一次 Package schema
  records/index；删除冲突带来的提前重复插入，同时保留 F268 schema closure。
- `test-runner/tests/package_service_contract_deployment.rs`：组合 F268 普通 test-service、
  `topLevel` 与 environment profile/schema closure 断言，以及 F267 sequence、stream、
  fresh-helper 负路径。删除旧 config-literal 正路径，不恢复 consumer overlay；fresh-helper
  路径使用 `test_service` 和 `compile_package_project_for_test`。

### `15d9ab6`

两处内容冲突均保留当前最终模型，随后整个补丁成为空差异：

- `compiler/driver/authoring/package_publication/tests.rs`：保留集成树更新后的 canonical std
  build identity；完成实际组合投影后又统一校准为
  `skiff-package-build-v4:sha256:fb02ab8f45ecd20b6e5a4b870d6c1280a51e0690481c6916f93423e8ea666536`。
- `doc/reference/testing.md`：保留最终 sequence 规则，即每一步可声明自己的 request
  subset 和恰好一种 `respond`、`throw` 或 `stream` 结果。

### `2350f1c`

唯一内容冲突为 `compiler/source/src/type_resolution_model.rs`。对应回归测试同时保留：

- F273 nullable literal-union alias 的 canonical structured `format` 字段/断言；
- F268 跨 Package `std.http.HttpHeader` 的精确 `header` 字段、textual projection 和
  Package identity 断言。

## 编译闭合与范围

F268 触及的空 provenance fixture 已补齐 F271 必填
`direct_return_origins: Vec::new()`。此外仅按任务的显式编译闭合授权修改：

- `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs`
- `runtime/host/src/loader/assembly_admission/tests/full_chain.rs`

两处只增加空 `direct_return_origins`，没有改变 effect、origin、artifact 或断言。最终
相对基线范围为 104 个文件、4876 行新增、1129 行删除；全部路径属于三个输入提交的触及
集合、任务/结果文档或上述两个显式授权路径，范围比较没有额外路径。

规范 std build identity 已在 authoring、canonical-package 和
package-service integration 断言中统一为实际组合 artifact 的
`fb02ab8f45ecd20b6e5a4b870d6c1280a51e0690481c6916f93423e8ea666536`。

## 结构检查

- repository conflict marker：0。
- F271
  `reachable_and_direct_return_origins_are_normalized_independently`：1 passed，
  34 filtered out。
- F273
  `aliases_expand_exactly_through_callbacks_and_nested_structural_types` 已由 source lib
  全量测试选中并通过。
- F268 test service/topLevel 路径由
  `test_service_environment_profile_projects_over_the_exact_package_closure` 和
  `std_test_service_overlay_uses_its_exact_compiler_owned_std_closure` 实际选中。
- F268 schema loader 正路径
  `loader_hydrates_cross_package_schema_children_without_a_foreign_code_slot`，以及缺失/错配和
  cross-Package cycle 负路径均在 13 个 loader unit tests 中实际选中。
- package-service-host consumer test fixture 只使用 `subject/main`；已删除的
  `test_config_literals_are_exact_typed_and_test_deployment_owned` 不存在；
  `test-runner` 与 `test-services` 中不存在 `skiff.test-doubles.json`。

## 验证

以下任务要求的命令均在最终代码、fixture、Cargo.lock、platform source 和 runner
registry 状态运行：

- `cargo test -p skiff-compiler-source --lib`：300 passed，0 failed。
- `cargo test -p skiff-compiler-lowering --lib`：43 passed，0 failed。
- `cargo test -p skiff-runtime-loader`：13 unit + 2 integration passed，0 failed；
  doc tests 0。
- `cargo test -p skiff-runtime-eval --lib`：149 passed，0 failed。
- `cargo test -p skiff-syntax`：116 passed，0 failed；doc tests 0。
- `cargo test -p skiff-compiler --test std_package_imports`：7 passed，0 failed。
- `cargo test -p skiff-test-runner --test package_service_contract_deployment`：
  20 passed，0 failed，1 ignored。
- `node scripts/run-skiff-tests.mjs`：std 11、alias 6、Package/Service Host 4，全部通过；
  2 个 canonical source test entries 通过。
- `cargo check --workspace --all-targets`：通过。
- `cargo fmt --all -- --check`：通过。
- `git diff --check`：通过。

canonical source suite 仅离线复用了本机已有的 router dependency cache；临时
`router/node_modules` symlink 已在命令结束后删除。没有启动或修改共享 stable instance。
