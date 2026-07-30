# P5-F445G-R2 Timeout IR independent review result

状态：`FAIL`。

固定审计对象：

`dee2d0b5d67df9a6f3358d68ee835c7695680e21`

实现已完成 timeout / value block / concurrent plan 的 File IR、lowering、linked-program、
linker 转换和 File IR v9/v7/v2 identity 切代；但 linker 仍接受两类不满足当前语言和 source
plan 合同的伪造 File IR，另有一条 required negative test 被更早的错误遮挡。前两项属于
F445G/I3 的 fail-closed owner，不能交给后续 evaluator 在执行时兜底；第三项使现有 T17
验收收据不足。

## 1. Findings

### F445G-R2-01 — HIGH — linker 接受超出 safe-integer 范围的 timeout duration

位置：

- `syntax/src/ast.rs:384,430-448`
- `runtime/linker/src/linker/execution_validation.rs:101-106`
- `runtime/linker/src/linker/file_conversion/timeout_execution_tests.rs:213-257`

合同与实现不一致：

- language duration 的 canonical 上限是
  `MAX_SAFE_DURATION_MILLISECONDS = 9_007_199_254_740_991`；
- source producer 的 `DurationLiteral::checked_milliseconds()` 同时拒绝零、乘法溢出和超过
  safe-integer 上限的结果；
- File IR 使用裸 `u64 duration_ms`，因此 artifact admission 必须重新校验持久值；
- 当前 `validate_duration()` 只检查 `duration_ms == 0`。任何
  `9_007_199_254_740_992..=u64::MAX` 都会通过 linker。

可复现证据：

1. 从现有
   `runtime/linker/src/linker/file_conversion/timeout_execution_tests.rs::execution_file()`
   取得其余字段均合法的 unit；
2. 把 `StmtIr::Timeout.duration_ms` 或 `ExprIr::Timeout.duration_ms` 改成
   `9_007_199_254_740_992`；
3. 调用同文件 `link(&unit)`；
4. `validate_duration()` 的唯一分支为 `duration_ms == 0`，随后 source site 和引用校验均通过，
   因而 `link()` 返回 `Ok`。反搜 `runtime/linker/**` 没有第二个 duration 上界 gate。

现有 negative test 只覆盖零值，没有覆盖最大合法值、最大值加一或 `u64::MAX`。这不是纯测试
缺口：持久 artifact 可以绕过 source compiler，当前 admission 实际接受 source 永远不会生成的
duration。

最小修复边界：

- 在 `runtime/linker/src/linker/execution_validation.rs` 对 statement/value timeout 同时要求
  `1..=9_007_199_254_740_991`；
- 避免出现无约束、可漂移的第二套 magic number；若不能直接复用 syntax owner，至少由 artifact
  contract 暴露明确上限，并用跨层测试锁定与 syntax 上限一致；
- linker tests 增加：最大合法值接受，最大值加一和 `u64::MAX` 对 statement/value 两种 wrapper
  都拒绝。

### F445G-R2-02 — MEDIUM — execution source site 可指向当前 File IR 之外的 source module

位置：

- `runtime/linker/src/linker/execution_validation.rs:206-232`
- `runtime/linker/src/linker/execution_validation.rs:116,151-157`
- `runtime/linker/src/linker/file_conversion/timeout_execution_tests.rs:8-36`

当前 `validate_source_site()` 只确认 `span.source_id` 在 `unit.source_map.sources` 中“至少命中
一次”，然后返回这个数字。它没有确认：

- source id 唯一；
- 命中的 `SourceMapSource.module_path` 等于 `FileIrUnit.module_path`。

concurrent plan 与 lane 的一致性也只比较返回的 source id。因而一个伪造 unit 可以新增
`SourceMapSource { id: 1, module_path: "foreign.module", ... }`，再把 timeout、plan 或所有 lane
site 的 `source_id` 改为 `1`；其 offset、block/expression ref 和 generation 保持有效时，
`link(&unit)` 返回 `Ok`。

这丢失了 source plan 在进入 I3 前已经验证的 module/owner 归属。owner 由 enclosing executable
隐含，不需要在 wire 重复存储；但 module 仍可由 File IR 与 source-map entry 精确核对。当前
foreign source 会进入 linked program，后续异常位置和诊断可被错误归属。

最小修复边界：

- File IR execution validation 对 source id 做唯一解析，而不是 `.any(...)`；
- source-authored timeout/plan/lane site 必须命中当前 `unit.module_path` 的 source-map entry；
- 增加 foreign-module source id、重复 source id，以及 plan/lane 同 id 但 foreign module 的拒绝
  用例；
- 不需要修改 IR shape，也不需要在 runtime 重建 source semantics。

### F445G-R2-03 — MEDIUM / TEST-QUALITY — `tail_closure` negative 没有到达 tail closure validator

位置：

- `runtime/linker/src/linker/file_conversion/timeout_execution_tests.rs:327-359`
- `runtime/linker/src/linker/execution_validation.rs:42-57,77-94,163-172`

`tail_closure` corruption 分支先把 statement 1 替换成携带 invalid tail dependency 的
`StmtIr::Concurrent`，随后用 `remove(1)` 把这个 statement 移出 statement table并作为
`ExprIr::ConcurrentValue` 加入 expression table。但是 fixture entry block 仍保留
`StmtRefIr { statement: 1 }`。

`validate_body()` 在遍历 expression 前先验证所有 block statement ref。此时 statement table
长度已经从 2 变成 1，因而固定先返回：

```text
constant[0] block `entry` references missing statement 1
```

执行不可能到达 `validate_concurrent_plan()` 的：

```text
concurrent tail dependencies do not close over all prior lanes
```

测试末尾只断言 `link(&unit).is_err()`，所以该错误被误计为 tail-closure rejection。生产
`execution_validation.rs:163-172` 的 closure check 本身存在；finding 是任务要求的 T17
corrupt-tail 证据没有真正覆盖目标分支。

最小修复边界：

- 构造 invalid `ExprIr::ConcurrentValue` 时保留一份合法 statement table，或同步删除 entry block
  对 statement 1 的引用；
- 不再只断言 `is_err()`，而要断言 diagnostic 包含
  `tail dependencies do not close over all prior lanes`；
- 只需修改 linker direct test，不需要修改 production。

## 2. 逐项目标审计

### 2.1 持久 shape

`artifact-model/src/executable.rs` 和
`artifact-model/src/executable/concurrent_plan.rs` 已形成唯一 tagged shape：

- statement timeout：`durationMs + body + site`；
- value timeout：`durationMs + value + site`；
- sequential `ValueBlock`：`block + result`；
- statement concurrent 与 concurrent value 共用 `ConcurrentPlanIr`；
- lane kind 只有 `statement | serial | tail`，每 lane 都保存
  `sourceOrder + dependencies + body/tail + site`，plan 自身保存 site。

artifact 与 linked enum/struct 都使用 strict tagged serde；unknown lane kind、legacy duration
字段和额外 lane 字段已有拒绝测试。该项除 F445G-R2-01 的 duration value domain 外，shape
完整。

### 2.2 Lowering 只消费 source plan

`compiler/lowering/src/lowered.rs` 把
`PackageSourceModel::execution_semantics()` 传入完整 package lowering。

`compiler/lowering/src/function_lowering/execution.rs`：

- duration、produces-value、lane source order、kind、dependencies 和 source site 都取自
  `TimeoutSourcePlan` / `ConcurrentSourcePlan`；
- AST 只用于降低实际 body/tail，并核对 source statement 是否与 plan 的 statement/serial/tail
  shape 相符；
- owner 结束时 `validate_execution_plans_consumed()` 要求 plan 数量精确消费；
- standalone helper 没有 execution semantics 时遇到 execution syntax 会 fail closed。

没有从 AST 重新计算 duration、lane dependency 或 lane kind 的第二套 source semantics。

### 2.3 checked value、site 与 corrupt plan

已闭合：

- duration 零值；
- source site 必须是 authored、source id 存在、offset 精确且正向；
- block/expression ref 存在；
- block label 唯一；
- lane order 连续；
- dependency 严格递增、唯一且只指向前序 lane；
- statement plan 无 tail，value plan 恰有最后一个 tail；
- tail dependency 精确闭包全部前序 lane；
- unknown serde kind 和旧 File IR generation 拒绝。

未闭合：

- duration safe-integer 上界：F445G-R2-01；
- source-map module 归属和 source id 唯一性：F445G-R2-02。

### 2.4 linked-program / linker 职责

`runtime/linker/src/linker/file_conversion.rs` 在任何转换前调用 execution validation，随后对
timeout、ValueBlock、concurrent plan 和各 lane 做逐字段转换。linked plan 不重新推导
dependencies、kind、tail 或 source site；assembly code linker 也只继续处理原有 type/call
linkage。职责边界正确。

### 2.5 File IR generation 与 identity

原子变化已对齐：

| 项 | 当前值 |
| --- | --- |
| File IR schema | `skiff-file-ir-v9` |
| File IR format | `skiff-file-ir-format-v7` |
| opcode table | `skiff-opcode-table-v2` |
| identity prefix | `skiff-file-ir-v9:sha256` |

canonical identity preimage 本来包含 schema/format/opcode 和 executable bodies；新 wrapper/plan
因此进入 hash。identity prefix、exact golden、compiler output 和 stale-generation rejection
均已更新。

`PACKAGE_ARTIFACT_SCHEMA_VERSION`、`SERVICE_CONTRACT_SCHEMA_VERSION`、
`RUNTIME_ASSEMBLY_SCHEMA_VERSION` 及其 DTO production 文件没有变化。`dee2d0b5` 中
`CURRENT_STD_LOCAL_ABI` / `CURRENT_STD_SCHEMA_INDEX` 的 test golden 更新来自 ancestor F445C
interface identity normalization 暴露的既有 fixture debt，不是 timeout 改写这些顶层 schema。
`runtime/linked-program/src/shared_image/tests.rs` 的
`global_ingress -> gateway_ingress` 同样是既有 test fixture 对当前 model 的机械同步。

### 2.6 public callable 与 maySuspend

timeout 没有新增 callable、service API 或 throws surface。`suspend_analysis.rs` 对 timeout 透明，
对 ValueBlock / concurrent value 合并 body 与 tail 的既有 call-graph 事实；unsuspending timeout
body 的 Local ABI 与 plain body 相同、package build identity 不同的真实 package compile test
已通过。

### 2.7 职责与文件规模

新增 production 文件：

| 文件 | 行数 |
| --- | ---: |
| `compiler/lowering/src/function_lowering/execution.rs` | 269 |
| `runtime/linker/src/linker/execution_validation.rs` | 233 |
| `artifact-model/src/executable/concurrent_plan.rs` | 64 |
| `runtime/linked-program/src/linked/concurrent_plan.rs` | 38 |

最大新增文件 372 行是 linker negative/round-trip tests，不是多职责 production。artifact 与
linked mirror 的重复是既有边界分层所需；未发现新的千行文件、明显可合并的第二套 plan 推导或
职责混杂。

### 2.8 T02–T04 / T17 本层测试

新增 compiler test 从真实 package source 经过 source model 和 lowering，覆盖：

- statement timeout；
- sequential timeout value 与 tail type；
- timeout 包 concurrent value；
- statement/serial/tail lane、dependency 和 source order；
- user `ValueBlock` 的真实 body/result；
- standalone 缺 plan fail closed；
- Local ABI 与 build identity 分离。

artifact、linked-program、linker tests 覆盖 strict serde、真实转换、v8/v6/v1 rejection、零
duration、synthetic/unknown/inexact site、lane order/dependency/tail/body corruption。不是只测
构造器；但当前 `tail_closure` 子分支实际被 stale statement ref 提前截断，见
F445G-R2-03。

缺少的 negative 包括两个 findings 所列 admission domain，以及一条被错误前置条件遮挡的
tail-closure direct test。

## 3. 旧 File IR 反搜分类

- Router 的两个 current consumer 已由 F445G-R1 单独迁到 v9；其 implementation
  `a4b1926d`、result `a1ad5a3e` 已给出 direct Router tests，不在本 review 重复记 finding。
- `cross-system-fixtures/dynamic-build-id-parity/case.json` 同时保留 File IR v8、format v1 和
  opcode v1，只被 ignored 的官方 identity fixture regenerator 读取；它不是当前 File IR
  admission positive consumer，属于显式跨 generation identity corpus。
- 其它 v3/v5/v7 字面量位于 legacy rejection、identity mutation或不经过 File IR admission 的
  in-memory fixtures；本次没有发现除已交 F445G-R1 的 Router 点之外的 active current reader
  仍接受 v8。

## 4. 独立验证

全部使用独立
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-r2-ir-review/build/cargo-target-review`：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model timeout_and_concurrent_file_ir -- --nocapture` | PASS，2/2 |
| `cargo test -p skiff-compiler --test timeout_artifact_lowering -- --nocapture` | PASS，4/4 |
| `cargo test -p skiff-runtime-linked-program --test timeout_execution -- --nocapture` | PASS，1/1 |
| `cargo test -p skiff-runtime-linker timeout_execution -- --nocapture` | PASS，4/4 |
| `cargo test -p skiff-artifact-identity suspension_generations_are_atomic_and_unrelated_domains_remain_stable -- --nocapture` | PASS，1/1 |
| `cargo test -p skiff-artifact-identity file_ir_identity_validation_rejects_non_current_generation_even_when_recomputed -- --nocapture` | PASS，1/1 |
| `cargo check -p skiff-compiler` | PASS |
| `git diff --check dee2d0b5^ dee2d0b5` | PASS |

没有运行 full gate、stable、live 或 network。

## 5. 退出条件

F445G/I3 在以下三项完成并由聚焦 tests 复验前不能独立验收：

1. linker 拒绝超过 safe-integer 上限的 statement/value timeout duration；
2. execution source site 精确属于当前 File IR module，且 source id 不可歧义；
3. `tail_closure` negative 精确命中 tail dependency closure diagnostic。

前两项只需修改 linker validation 与其直接 tests（若抽取共享 duration 上限，可增加最小
artifact contract owner）；不需要改 IR shape、重新升 File IR generation，或改 source plan、
eval、host、Router。第三项只修改现有 linker direct test。
