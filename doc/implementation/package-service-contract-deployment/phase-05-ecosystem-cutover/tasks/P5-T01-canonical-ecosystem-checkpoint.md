# P5-T01：Canonical Authoring / Storage / Control Checkpoint

> Superseded scope note（2026-07-29）：本任务是历史shared checkpoint。下文将`contract.yml`、
> `deployment.yml`和`assembly.yml`列为正向authoring的部分已被current authority废止，不得重新执行或作为
> completion criterion。ServiceContract、ServiceDeployment与RuntimeAssembly现均由tooling生成；
> RuntimeAssembly roots来自watch registry、显式roots/receipts或平台部署状态。

## 权威输入、风险与证据状态

- 唯一设计：`doc/architecture/package-service-contract-deployment.md` §1–§5、§9–§14。
- 执行决策：本阶段 `phase-plan.md` §1–§2、§6、§10。
- 风险/验收组：高风险authoring、immutable storage、release/CAS、router↔runtime control wire；R01。
- 当前成熟度：planning docs checkpoint；完成后仅是shared implementation checkpoint。
- 证据对任一artifact schema/path/identity、pointer/control wire、resolver、fixture或Cargo/lock改动失效。

## DAG 与执行约束

- 依赖：P5-D01 PASS。解锁：R01；R01 PASS后才启动T02–T05。
- branch：`codex/p5-t01-canonical-ecosystem-checkpoint`。
- worktree：`/Users/geek/workspace/skiff-p5-t01-ecosystem-checkpoint`。
- 使用新的开发Agent；证据只锚定任务source commit与合入后的exact Skiff integration tree。
- 五分钟内产生第一个真实code edit；此前不跑测试、不重做设计。无法在四对象之外
  零aggregate的前提下冻结接口时，回报 `TASK_NOT_EXECUTABLE` 及精确缺口。

## 目标与写入范围

建立最小新模块/强类型fixture，冻结但不迁移consumer：

- `package.yml contracts`、`contract.yml`、`deployment.yml`、`assembly.yml` 的strict authoring DTO/
  parser boundary；精确字段记录在聚焦reference/test fixture，不复制canonical artifact body。
- PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly各自的typed immutable
  record/path/reader/writer及pointer操作；不引入common artifact-kind enum/envelope。
- environment activation state的committed/pending、activationId、participant set、prepare/commit/abort CAS、
  crash recovery与strict identity/path validation；它是operational record，不进入四对象artifact hierarchy。
- router↔runtime的exact prepare/ACK/reject/commit/abort/register wire fixture：environment/activationId/
  expected+candidate generation/assembly/replica，无service/build target。
- production `RuntimeAssemblyContentResolver` 对四种immutable record的strict解析；不调用host admission。

允许对 `artifact-model`、`artifact-identity`、compiler input/contract、`deployment`、新storage模块、
`runtime/loader`及两端protocol fixture做最小public seam；T01独占root Cargo/lock。禁止修改现有
CLI/dev-sync/router dispatch/runtime host/test-runner consumer，禁止删legacy module或修旧测试来伪造绿灯。

## 完成态与最早探针

1. 四类record往返bit-identical，unknown/missing/tampered/cross-root/duplicate 全部fail closed。
2. prepare CAS只创建pending；任一resolver/admission reject、participant disconnect或abort保持committed
   tuple byte-identical。全部participant exact ACK后才commit单调generation；commit后通知/重启可幂等向前收敛。
3. cross-language golden fixture在Rust/TypeScript两端解码同一state/control wire；mutation能检出旧
   `artifactRoots/serviceConfig/serviceId/buildId/target`字段。
4. production resolver从exact RuntimeAssembly ref闭合加载deployment/contract/package/file/resource，
   不读旧pointer/index、不修复identity、不反序列化成类型前查raw JSON。
5. contract-first probe在provider artifact不存在时仍能从ServiceContract构造合法package
   compile input；deployment/provider字段混入contract被拒绝。

## 唯一聚焦验证 owner

```bash
cargo test -p skiff-artifact-model -p skiff-artifact-identity -p skiff-deployment
cargo test -p skiff-runtime-loader runtime_assembly
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
git diff --check
```

只格式化本任务Rust文件；不跑完整compiler/runtime/router/checks/verify。

## 回报

提交一个commit并合入Skiff integration branch，回报exact authoring/file/path接口索引、activation
state machine/崩溃点表、wire字段表、生产resolver、
legacy反向搜索、测试与自验收矩阵。
