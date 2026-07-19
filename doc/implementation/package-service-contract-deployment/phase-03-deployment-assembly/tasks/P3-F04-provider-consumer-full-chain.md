# P3-F04：Provider / Consumer Full-chain Evidence

## 权威输入、失败与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§5、§9、§10、§12、§14。
- 执行输入：P3-A01 对 production candidate `34b6a863534b435d1e81b88de4cd8c0ed8a352fa` 的
  blocking finding A01-12：现有 `full_chain.rs` 只有 provider，consumer requirements、service call refs 与
  activation-relative service edge 均为空。
- 风险/验收组：中风险 integration evidence repair；不得修改 production schema、projection、resolver、loader、
  linker或 admission 语义。
- 有效证据状态：旧 full-chain/T09 integration evidence失效；其它分层测试及 foundation/compiler证据保持有效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent合流后统一重建受影响证据。

## DAG 与执行约束

- 依赖：A01初次验收 FAIL；可与 F05并行。
- 解锁：T09R affected-gate rebuild。
- branch：`codex/p3-f04-provider-consumer-e2e`。
- worktree：`/Users/geek/workspace/skiff-p3-f04-provider-consumer-e2e`。
- 首先编辑真实 fixture/test，不先扩大测试或重做设计。若真实 producer无法表达 consumer service edge，回报
  `TASK_NOT_EXECUTABLE` 与 exact schema/owner，不在测试中手工绕过 projection/resolver。

## 写入范围与完成态

- 只修改 `runtime/host/src/loader/assembly_admission/tests/full_chain.rs` 及同目录仅供该测试使用的 helper。
- fixture必须包含 canonical `ServiceContract`、provider与consumer `PackageArtifact`；consumer显式带
  `ServiceRequirement` 与 `ServiceCallRef`，并经真实 `ServiceDeploymentInput -> projection -> resolution -> typed
  load/link -> admission` 全链路。
- consumer edge不得手工写入 resolved deployment/assembly，不得使用 legacy aggregate builder、fake空 contract、
  display-name/path猜测或 test-only fallback。
- admit后从 active candidate按 consumer activation + caller build + service slot验证 exact provider binding，再按
  `ServiceContractRef + ContractOperationId`取得 canonical descriptor/value plan。
- 同一测试证明 contract descriptor仍由 canonical store拥有、active lookup不增加 resolver/artifact I/O；tampered
  reload仍保留旧 active assembly。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-host --lib projected_nonempty_assembly_admits_and_active_lookup_is_io_free
git diff --check
```

若需要更名测试以准确表达 provider/consumer链路，可同时运行新 exact filter；不得运行完整阶段 gate。

## 回报

提交一个 commit，回报 commit、fixture中的 provider/consumer/service edge、真实 producer调用链、active
activation-relative lookup断言、resolver I/O计数、failed reload断言与命令结果。

