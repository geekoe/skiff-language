# P5-F268I Public test service integration

状态：Ready。

## 直接父节点与权威链

- 直接父节点：
  `P5-F268-public-repository-test-service-migration-result.md`
- 父任务：
  `P5-F268-public-repository-test-service-migration.md`
- 该父任务依赖 F265–F267；其测试语义最终追溯到
  `doc/reference/testing.md`。

本节点只负责把父节点已经完成的 Skiff 实现合入当前 Phase 5 集成代码，不改变测试模型。

## 精确输入与 DAG 位置

- 集成基线：
  `codex/package-service-phase-05@63565894`。
- F268 非重复代码提交，按顺序：
  `5ca8534`、`15d9ab6`、`2350f1c`。
- `3aad2c6` 的 result 文档已经以 `63565894` 合入，不再重复。
- F268 分支中的 `be4213f`、`d4cd72a`、`a878c4e` 与集成分支已有提交
  patch-equivalent，不得重复取入。
- 完成后解除：F269 的 Internals test-service 总验收与 F270 legacy removal。
- 当前成熟度：实现检查点；完成后仍只是预验收候选，不冻结稳定周期。

## 已知冲突与必须保留的共享状态

在上述基线上试取 `5ca8534` 已确认十个内容冲突：

- `compiler/lowering/src/lowered.rs`
- `compiler/lowering/src/publication_local_refs.rs`
- `compiler/projection/src/package_artifact/callables/normalization.rs`
- `compiler/source/src/dependency_analysis.rs`
- `compiler/source/src/type_resolution_model.rs`
- `compiler/tests/std_package_imports.rs`
- `runtime/eval/src/program_stream.rs`
- `test-runner/fixtures/package-service-host/consumer-tests/main.test.skiff`
- `test-runner/src/canonical_store.rs`
- `test-runner/tests/package_service_contract_deployment.rs`

冲突解析必须组合语义，不能整文件取一侧。当前集成分支已经拥有并必须保留：

- F267 最终 inline effect setup/body、typed wire snapshot、sequence 和 case finalization；
- F271 结构化 caller projection、`returnOrigins` /
  `directReturnOrigins`、fresh root/payload 与环检测；
- F272 stable nested field narrowing；
- F273 public alias 递归展开及 canonical structured IR。

F268 需要新增且必须接入上述最终模型：

- 普通 `kind: test` service runner/authoring；
- `access: topLevel` 的精确函数、类型、常量和执行 signature 链；
- canonical std Package effect target；
- 跨 Package schema closure 的 loader hydration；
- 公共仓库测试服务 fixture 与旧 runner 输入删除。

原 F268 owner 的一次性冲突语义交接是本任务的补充输入；若交接与代码或父节点冲突，以父节点和当前最终模型为准。

## 写入范围

- 工作目录：
  `/Users/geek/workspace/skiff-p5-f268-integration`
- 分支：
  `codex/p5-f268-integration`
- 只允许修改三个被取入提交触及的 Skiff 文件及本任务 result。
- 可以为冲突组合补最小回归测试；不得顺手修父 result 已列出的九类历史 fixture。

### 已确认的基线编译闭合

F271 新增必填 `direct_return_origins` 后，当前集成基线还有两个未被 F268 触及的 Host
测试 fixture 漏初始化。为使本任务拥有的 `cargo check --workspace --all-targets` 可执行，
额外授权且只授权：

- `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs`
- `runtime/host/src/loader/assembly_admission/tests/full_chain.rs`

两处都必须补 `direct_return_origins: Vec::new()`；fixture 的既有 provenance 为空，不得同时
改变其它 effect、origin、artifact 或断言。

## 非目标与禁止事项

- 不修改 `skiff-packages` 或 Internals。
- 不重写 F271/F273 的公共 artifact/type 语义。
- 不恢复 overlay、`root.*` dependency access、外部 doubles manifest 或旧 config literal。
- 不操作 stable、不访问外网、不 push。
- 不把冲突简单解析为 `ours` 或 `theirs` 后依靠测试碰运气。

## 完成标准与证据 owner

开发 Agent 拥有以下聚焦/便宜集成证据；命令必须在最终提交状态运行：

```bash
cargo test -p skiff-compiler-source --lib
cargo test -p skiff-compiler-lowering --lib
cargo test -p skiff-runtime-loader
cargo test -p skiff-runtime-eval --lib
cargo test -p skiff-syntax
cargo test -p skiff-compiler --test std_package_imports
cargo test -p skiff-test-runner --test package_service_contract_deployment
node scripts/run-skiff-tests.mjs
cargo check --workspace --all-targets
cargo fmt --all -- --check
git diff --check
```

另外必须做结构检查：

- 所有 conflict marker 为零；
- `directReturnOrigins` 与 alias canonical IR 测试仍存在并通过；
- F268 新 test-service/topLevel/schema-loader 正负测试均实际被选中；
- 旧 overlay/config-literal/doubles 正路径没有被冲突解析恢复。

提交代码和
`P5-F268I-public-test-service-integration-result.md`，报告每个冲突的组合方式、最终提交范围和
精确测试计数。证据在代码、fixture、Cargo.lock、platform source 或 runner registry 改变后失效。

若 5 分钟内无法确定某个冲突的语义 owner，返回 `TASK_NOT_EXECUTABLE` 和该文件的两侧事实，
不得自行选择公共语义。
