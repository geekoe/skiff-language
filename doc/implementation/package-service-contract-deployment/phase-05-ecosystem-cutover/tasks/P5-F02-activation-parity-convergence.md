# P5-F02：Activation Parity Convergence

## 权威输入、风险与证据状态

- 唯一设计：`doc/architecture/package-service-contract-deployment.md` §1–§5、§9–§14；执行输入为
  `phase-plan.md`、P5-T01/F01、两次P5-R01 FAIL及P5-D02有界审计。
- source code锚点：`128af4a7d638026b90827c73f902c2dd66f3e79a` / tree
  `85773fa60ba02711e9313a2a5c7dbbbe97dd229b`；任务分支从包含D02/F02合同的最新integration HEAD创建。
- 风险/验收组：高风险shared raw/typed codec repair；不改变四对象、activation transaction或consumer DAG。
- 当前成熟度：R01第二次FAIL已触发熔断，D02已收齐同类缺口。F02一次关闭全部remaining findings，
  combined probe通过后才允许第三次R01窄复验。
- path codec与alias owner证据保持有效，禁止重开；activation request/state/control的全部Rust/TS parity证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：P5-D02完成。解锁：combined repair probe；通过后原R01 reviewer第三次窄复验。
- branch：`codex/p5-f02-activation-parity-convergence`。
- worktree：`/Users/geek/workspace/skiff-p5-f02-activation-parity`。
- 使用新的开发Agent；五分钟内产生第一次真实code edit，不先重跑旧测试。
- 若必须改变transaction字段、四对象、T02–T04职责或引入新的domain artifact，立即回报
  `TASK_NOT_EXECUTABLE`，不得自行扩设计。

## 写入范围与单一owner

允许修改：

- `artifact-model` activation request/control DTO、共享token/generation/runtime-assembly identity lexical leaf及tests；
- `artifact-identity`仅改为消费上述runtime-assembly identity leaf owner及直接tests；
- `deployment/src/storage/activation.rs`和直接tests，仅修required-nullable state decode与共享规则消费，
  不改CAS/state machine；
- `router/src/protocol/assemblyActivationProtocol.ts`、新建独立strict raw JSON/leaf codec模块及直接tests；
- `cross-system-fixtures/package-service-ecosystem/**`共享raw corpus与runner；
- 必要Cargo/lock仅限确有依赖需求。优先用冻结ASCII值域避免新增Unicode category依赖。

不得修改coordinate/path、alias、CLI/router coordinator/runtime host/transport/test-runner或任何T02–T05
consumer，不恢复legacy字段。

`extra-review`维护性约束：Rust control文件已超过500行、TS protocol文件超过400行。raw tokenizer、
value-domain leaf和fixture corpus helper必须按职责拆到独立模块；不得把新parser继续堆进现有DTO/dispatcher文件。
三个重复mutation applicator收敛为同一raw corpus/case runner。行数本身不是gate，职责混合与重复owner才是。

## 冻结值域与raw trust boundary

1. `environment`固定为ASCII `[A-Za-z0-9._-]{1,200}`，并拒绝`.`、`..`。
   `activationId`、`replicaId`、participant id固定为ASCII visible token `[\x21-\x7e]{1,200}`。
   这些是operational identifier而非display text；因此BOM、NBSP、ZWSP、Cc/Cf、所有非ASCII、paired/lone
   surrogate与NFC/NFD样本全部一致拒绝。排序按ASCII/UTF-8 bytes，Rust/TS结果相同。
2. raw generation lexeme只允许`0|[1-9][0-9]*`；拒绝sign、`-0`、decimal、exponent、rounding与underflow。
   typed TS decoder另用`Object.is(value, -0)`拒绝negative zero并要求safe integer。
   committed/register generation范围是`0..=9007199254740991`；request及所有transition的
   `expectedGeneration`最多`9007199254740990`；candidate必须精确等于expected+1，允许到MAX_SAFE。
3. `pending`是required nullable：显式`null`合法，字段缺失非法。participant array必须dense、nonempty、
   exact unique、按canonical bytes升序；sparse hole与non-string失败。
4. runtime assembly identity的prefix与64位lowercase SHA-256 lexical检查只有artifact-model leaf owner；
   artifact-identity内容/路径验证消费它，不复制prefix/hex循环。
5. reject reason保持现有五个闭集；request/state/control各variant的top/nested exact fields、type discriminator、
   schemaVersion都由真实decoder验证。
6. TS新增production可消费的raw入口，接收string或bytes：bytes使用fatal UTF-8；raw parser在值被转换前拒绝
   top/nested/escaped duplicate key、非法Unicode scalar与非canonical number lexeme，再调用typed decoder。
   不允许先`JSON.parse`后声称覆盖raw trust boundary。Rust request/control public deserialize及state production
   `parse_state`必须对同一raw corpus给出相同outcome。

## 共享证据与完成态

- 新的raw corpus明确记录`target/request|state|control`、原始UTF-8 text或byte hex与`accept|reject`；同一case
  实际进入Rust和TS raw decoder。至少覆盖D02 checklist：FEFF/NBSP/ZWSP/Cc/Cf/Unicode/surrogate、200/201
  边界、`-0`/fraction/exponent/rounding/underflow/MAX_SAFE边界、duplicate top/nested/escaped key、非法UTF-8、
  missing pending/null positive、dense/sparse participant、reject/register/discriminator/nested assembly与五reason。
- object-level typed tests另覆盖TS无法从raw来源保留的`Object.is(-0)`和sparse arrays。
- request/control仍只有public seam/fixture，无提前consumer迁移；T02、T03、T04可分别直接消费同一冻结入口。
- `verify.mjs --self-test`运行完整corpus；另提供由主integration owner运行、开发Agent不得预跑的
  `--combined-probe`最小回归集，至少包含FEFF、raw duplicate、rounded generation、missing pending、sparse array。

## 开发Agent唯一聚焦验证

```bash
cargo test -p skiff-artifact-model assembly_activation
cargo test -p skiff-artifact-identity runtime_assembly
cargo test -p skiff-deployment activation
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
git diff --check
```

不运行`--combined-probe`、router完整type-check、runtime/compiler/checks/verify/live。提交一个commit，回报
`D02 checklist | Rust raw/typed owner | TS raw/typed owner | shared case | test`矩阵、exact commit/tree、反向搜索、
维护性拆分说明及worktree clean。
