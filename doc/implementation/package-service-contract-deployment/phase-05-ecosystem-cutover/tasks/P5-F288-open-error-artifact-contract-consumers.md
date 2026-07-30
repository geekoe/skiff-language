# P5-F288 Open error artifact and contract consumers

状态：Ready for contract review。

## 直接父节点与权威链

- A1冻结结果：
  `P5-F284-open-error-model-acceptance-result.md`
- 实现owner父结果：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`
- 设计父结果：
  `P5-F279-open-service-error-channel-design-result.md`

启动时只读本任务；需要依据时沿父链向上读取。F284冻结的DTO不得由本任务修改。

## DAG位置、基线与共享状态

- 节点：F280 `W2-A Artifact, contract and dependency consumers`。
- production base：当前integration中已通过A1的strict model checkpoint。
- 输入：
  - `PackageCallableSignature`已没有`throw_types`；
  - `BoundaryOperationContract`已没有`errors`；
  - model schema已切File IR v6/format v4、PackageArtifact v4、ServiceContract v4、definition v3；
  - artifact identity marker/prefix尚未同步切换。
- 并行节点：
  - F287只改std/prelude/tooling；
  - F285只改有界package-call type resolution；
  - 未来W2 language叶子只在F285 result落盘后建档，并在派发前重新审计写入面；
  - 后续runtime不得在本任务完成前实现自己的artifact identity fallback。
- 完成后解除：W2-R runtime loader/index/channel与W2 language/artifact combined compile。

## 唯一production写入范围

- `artifact-identity/**`
- `compiler/compiled/**`
- `compiler/projection-input/**`
- `compiler/projection/**`
- `compiler/contract/**`
- `compiler/input/src/contract_dependencies/**`
- `compiler/driver/source_compile/canonical_dependencies.rs`
- `deployment/**`
- `test-runner/src/package_schema_contract.rs`

允许机械更新上述owner的co-located fixtures/goldens。禁止修改artifact-model/runtime-model冻结DTO、
compiler source/lowering、test-runner其它production、runtime loader/eval、std、router、telemetry、
skiff-packages或internals。

## 完成标准

### 1. 删除closed throw-set consumer

- compiled/projection-input不再生产、复制或验证callable throw types；
- public/topLevel callable projection只处理parameters、return、maySuspend与独立semantic facts；
- dependency identity rebinding不再遍历throw types；
- boundary projection不再生成0/1/N error contract或structural error union；
- contract normalization/existential validation/schema closure不再读取operation error；
- deployment eligibility不再编译error value plan；
- compiler input与test-runner schema closure不再从operation error收集Package type；
- 删除只为closed set存在的helper、diagnostic、fixture与golden，不保留empty/default shim。

必须保留并不改义：

- throw `payload_type`；
- throw provenance/`throws_caller_alias`；
- `BoundaryEffectGuarantee.detached_error`；
- F278 same-heap facts与eligibility。

### 2. Identity与strict generation同步

在唯一artifact-identity owner中同步bump并验证：

- `FILE_IR_IDENTITY_PREFIX`：v5 → v6，对应File IR v6；
- `PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER`：v2 → v3；
- `PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX`：v4 → v5；
- `PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER`：v1 → v2；
- `PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX`：v3 → v4；
- `SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER`与`SERVICE_PROTOCOL_IDENTITY_PREFIX`：v3 → v4；
- 任何直接把上述canonical projection schema写入preimage的版本marker。

必须证明：

- 不修改legacy `PACKAGE_BUILD_IDENTITY_*`与`PACKAGE_LOCAL_ABI_IDENTITY_*`常量；
- `ContractOperationId`不变；
- Publication/Operation ABI identity不因本任务新增throw set；
- PackageSchemaTypeId与PackageSchemaIndexIdentity算法不变；
- 未来实现可能抛出类型的变化不进入Local ABI或ServiceProtocolIdentity；
- 仅引用上游新identity的assembly/deployment/build自然变化时，不为其复制无关schema bump。

### 3. Strict admission与golden

- 新validator只接受A1新schema与新identity generation；
- `artifact-identity/src/file_ir.rs::validate_file_ir_identity`必须拒绝非当前File IR
  schema/format/opcode，不能按输入携带的任意旧version重新计算后接受；
- 旧`throwTypes/errors`已由model strict拒绝，consumer不得用serde default/legacy reader重建；
- stale旧identity prefix、旧preimage、owner/key/type id错配继续fail closed；
- 更新golden前必须用mutation test证明每个预期改变/不变的identity domain，不盲改字符串；
- reverse search确认本任务owner中没有closed throw-set路径。

## 最早风险探针与验证owner

至少增加/更新：

- empty/public callable signature serialization不含`throwTypes`；
- service operation serialization不含`errors`；
- implementation body可能throw的变化不改变Local ABI/ServiceProtocolIdentity；
- parameter/return/stream/callback/schema变化仍改变应有identity；
- old schema/prefix与stale artifact严格拒绝；
- contract closure只含parameter/return/stream/callback所需roots；
- F278 same-heap identity正负结果不变。

本任务的strict admission结论只覆盖Rust artifact/contract owner。Router的
`router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`仍硬编码File IR v5、Package artifact build
v4与ServiceProtocol v3，并独立校验protocol identity；它不在本任务范围。后续W2-W transport/router任务必须
显式依赖F288，迁移该loader及相关protocol/manifest validators并刷新tooling golden，仍禁止legacy fallback。

本任务唯一拥有以下聚焦验证；先用`--list`确认非零：

```bash
cargo test -p skiff-artifact-identity --lib --no-fail-fast
cargo test -p skiff-compiler-compiled --lib --no-fail-fast
cargo test -p skiff-compiler-projection-input --lib --no-fail-fast
cargo test -p skiff-compiler-projection --lib --no-fail-fast
cargo test -p skiff-compiler-contract --lib --no-fail-fast
cargo test -p skiff-compiler-input contract_dependencies --no-fail-fast
cargo test -p skiff-deployment --lib --no-fail-fast
git diff --check
```

若某crate因尚未合流的language/runtime consumer断编译，记录精确依赖并运行仍可执行的owner subset；不得为了
让全workspace编译修改未授权文件。完整combined compile由所有W2 compiler/artifact节点合流后的唯一owner
运行。

## 风险、禁止范围与交付

- 风险：高；验收组`A3-artifact-contract`。
- 不修改A1 DTO、不实现throws语法、不增加error type list、不实现runtime envelope/stack。
- 不恢复old schema/identity兼容，不用display/shape推断。
- 不修改F278 same-heap语义或boundary availability阈值。
- worktree：`/Users/geek/workspace/skiff-p5-f288-error-artifact`
- branch：`codex/p5-f288-error-artifact`
- 不push，不操作stable。
- 从启动到第一次production修改不超过5分钟；不可执行时返回`TASK_NOT_EXECUTABLE`。
- 完成后提交并返回commit、identity mutation矩阵、可执行/被遮挡测试与设计缺口；不得自行承接runtime。
