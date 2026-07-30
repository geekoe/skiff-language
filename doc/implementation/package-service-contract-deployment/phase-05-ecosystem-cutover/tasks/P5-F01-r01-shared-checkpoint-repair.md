# P5-F01：R01 Shared Checkpoint Repair

## 权威输入、失败分类与证据状态

- 唯一设计：`doc/architecture/package-service-contract-deployment.md` §1–§5、§9–§14。
- 执行输入：`phase-plan.md`、P5-T01、P5-R01，以及R01在integration commit
  `0cebf349c3e957fd82b3515b07dad28a8b148838` / tree
  `f37e33660f97f17dce20014d901537ac98f03178` 的三个blocking findings。
- finding 1是既定设计下的实现缺陷：coordinate codec非单射，typed pointer/CAS可能碰撞。
- finding 2是缺失shared owner：tooling→router activation request、TS activation-state codec及Rust/TS
  wire值域未冻结。
- finding 3是重复owner：dependency alias lexical/reserved规则同时存在于artifact-model与compiler。
- 三项都不需要新的架构或用户决策；由同一个T01 shared checkpoint repair owner一次关闭。
- T01关于path/storage、authoring alias、activation state/control wire及其fixture的证据失效；其它已通过
  的四对象模型/identity和consumer未迁移证据保持有效，除非本任务越界修改。
- integration边界：只提交task branch，不merge integration/main、不push；主Agent合流并执行combined
  probe后，由原R01 reviewer只复验这三个exact blockers。

## DAG 与执行约束

- 依赖：R01@`0cebf349` FAIL。解锁：R01窄复验；PASS后才解锁T02–T05。
- branch：`codex/p5-f01-r01-shared-checkpoint-repair`。
- worktree：`/Users/geek/workspace/skiff-p5-f01-r01-repair`。
- 使用新的开发Agent；五分钟内产生第一次真实code edit，不重做设计、不迁移consumer。
- 无法在下述shared seam内关闭任一finding时回报`TASK_NOT_EXECUTABLE`及精确缺口，不把schema留给
  T02/T03/T04分别发明。

## 写入范围与冻结决策

允许修改T01直接owner及其最小consumer证明：

- `artifact-identity/src/ecosystem_paths.rs`、`deployment/src/storage/**`及直接tests；
- `artifact-model/src/ecosystem_authoring.rs`、`assembly_activation_control.rs`、必要`lib.rs`导出；
- `compiler/input-model/src/dependencies.rs`及直接tests，仅用于改为消费shared alias leaf owner；
- `router/src/protocol/assemblyActivationProtocol.ts`；
- `cross-system-fixtures/package-service-ecosystem/**`；
- path变更直接影响的`runtime/loader` fixture/test。除非编译所需，不修改Cargo/lock。

不得实现router coordinator、CLI/watch、runtime host/transport、test-runner或legacy删除，不修改
T02–T05其它production consumer。

本repair冻结以下公共边界：

1. coordinate codec必须可证明单射、确定且path-safe。保留“原始`~`拒绝”前提，`.`与`/`使用互不为
   前缀且可逆的escape token（fixture记录最终精确拼写）；`a.b/c/d`与`a.b/c..d`必须得到不同路径。
   record、pointer、resolver与mutation tests共同消费同一个codec owner，不允许靠上游validator偶然排除
   collision pair。
2. tooling→router请求固定为一个strict `AssemblyActivationRequest`：
   `schemaVersion/environment/activationId/expectedGeneration/assembly`；candidate generation由router从
   expected generation单调推导，participant set由router冻结，请求不得携带service/build/target/root。
   TS protocol导出canonical control endpoint常量`POST /__skiff/activate-assembly`与request decoder，供
   T02/T03共同消费；本任务不实现endpoint。
3. TS protocol增加strict `EnvironmentActivationState` codec，与Rust durable state的
   `schemaVersion/environment/committed/pending`精确一致。state、request、control三类Rust/TS decode均执行
   相同值域：identifier/token非空、无首尾空白/控制字符、最长200 bytes；generation为
   `0..=Number.MAX_SAFE_INTEGER`；assembly ref满足canonical RuntimeAssembly identity；unknown/missing/
   duplicate/unsorted participant与代际不变量均fail closed。不能只提供调用者可忘记调用的`validate()`。
4. golden/mutation fixture由Rust与TypeScript实际decoder共同消费，而不是只核对手写field-name数组；至少覆盖
   empty/whitespace token、negative/fractional/overflow generation、invalid assembly identity、unknown/
   missing field、duplicate/unsorted participants及旧per-service字段。
5. package与contract dependency alias的lexical/reserved判定只有一个shared leaf owner。
   compiler现有公开helper若需保留，只能薄委托该owner；artifact authoring和compiler input使用同一组
   positive/negative vectors，禁止复制字符循环或reserved列表。

## 完成态与聚焦验证 owner

- collision pair及相邻escape mutation证明record/pointer path唯一，immutable write/CAS不会cross-coordinate。
- request、state、control fixture在Rust/TS两端对同一正例成功、对同一mutation corpus失败。
- T02只需序列化shared request并调用冻结endpoint；T03只需消费同一decoder并协调state；T04只需消费同一
  Rust state/control类型，不再协商字段或值域。
- alias规则源码反向搜索只剩shared owner与薄委托，不存在第二reserved集合/字符validator。

```bash
cargo test -p skiff-artifact-identity ecosystem_paths
cargo test -p skiff-artifact-model ecosystem_authoring
cargo test -p skiff-artifact-model assembly_activation_control
cargo test -p skiff-deployment activation
cargo test -p skiff-compiler-input-model dependencies
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
git diff --check
```

不跑T01整套crate gate、router完整type-check、runtime loader suite、checks/verify/live。提交一个commit，回报
`R01 finding | code owner | mutation/negative evidence | command`矩阵、exact commit/tree与worktree clean。
