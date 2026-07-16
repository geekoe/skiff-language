# Phase 01：Package Code Contract Foundation

状态：`ready`
前置：总体架构草案已存在；本阶段的 T01 负责把仍含混的 value contract 收敛为 canonical
契约。
阶段目标：在不切换 service runtime 形态的前提下，让 `PackageUnit` 成为足以支持后续
deployment projection 和本地 service assembly 的完整代码契约。

## 1. 阶段完成态

阶段验收时必须同时成立：

1. `package.yml` 可以在顶层声明 `services`，并通过单一 resolver 从可信 service artifact root
   解析精确 id/version/protocol contract；package 与旧 service source path 不各自维护一套规则。
2. 每个 package public callable 都有 typed Local Code ABI、sound may-effect、link requirements，
   以及显式的 boundary projection 状态。
3. boundary projection 使用 lane-scoped 即时 `LinkableValuePlan<Lane>`；只有跨
   request/persistent lane 才叠加
   `RecoverableValuePlan`。
4. request-scope `any I` 和 native handle 若允许跨 boundary，只能成为 callback capability；不能
   把 method table、native object 或本地 handle 当作 ordinary data。
5. mutable helper、依赖 caller heap alias 的函数仍可本地链接，但其 boundary 状态必须是带稳定
   reason code 的 `Unavailable`。
6. `PackageUnit` 的 build/local ABI/boundary ABI identity 输入有单一 owner，且不受诊断文本或
   artifact 文件路径影响。
7. package build/test 能读取、校验并保留这些新事实。运行时真正执行 service dependency 不在
   本阶段范围内。
8. 现有 service source production path 在本阶段仍能编译；它只能消费新的共同契约，不能复制
   新分析或 projection。

## 2. 非目标

- 不把 `ServiceUnit` 改成 config-only；这是 Phase 02。
- 不引入 Runtime Assembly 或 `InProcessBoundary` dispatch；这是 Phase 03。
- 不删除 router relay 或 remote transport；这是 Phase 04。
- 不迁移官方 package、私有 service 或全量 fixture；这是 Phase 05。
- 不为旧 artifact/manifest 增加兼容 reader。
- 不追求 effect analysis 对所有程序都完备；允许保守 `Unknown`/`Unavailable`，但不允许漏报。

## 3. Artifact 契约基线

下面是职责约束，不要求逐字采用同名 Rust struct：

```text
PackageUnit
  ├─ files / resources / package dependencies
  ├─ publicationAbi / export index       # Local Code ABI 的 canonical surface
  ├─ serviceRequirements[]
  ├─ callableContracts[]
  │    ├─ localCodeAbiRef                # 引用 canonical callable，不复制 signature
  │    ├─ effectSummary
  │    ├─ linkRequirements
  │    └─ boundaryProjection = Available(contract) | Unavailable(reasonCodes)
  ├─ packageBuildIdentity
  ├─ packageAbiIdentity          # Local Code ABI surface
  └─ boundaryAbiIdentity         # boundary projection surface
```

约束：

- `ServiceContractRequirement` 记录 alias、service id、精确 version、protocol/operation expectation；
  不记录 provider package id，不用 provider build id 寻址。
- `PublicationAbiUnit`/export index 继续作为 Local Code ABI 的唯一 signature owner；callable contract
  按稳定 callable/operation id 引用它。禁止再存一份独立生成、需要同步校验的 local signature。
- `Unavailable` 原因是稳定 enum/code；供人阅读的 detail 不进入 ABI identity。
- boundary identity 包含 operation id、signature、value/callback/stream/error plan 和 availability
  code；不包含 deployment identity、route、配置值或 diagnostic wording。
- effect 是 callable-level typed fact；禁止继续用 raw JSON 或 `EffectMetadata::Empty` 表达“尚未
  分析”。

## 4. DAG

```text
T00 service contract surface ─────────────────► T01, T03, T04
T01 boundary / recoverable canonical contract
 ├────────────────────────────────────────────► T03 artifact code contract
 └────────────────────────────────────────────► T07 boundary projection decomposition

T02 artifact identity decomposition ───────────► T03
T03 ───────────────────────────────────────────► T04 canonical service requirement input
T05 projection handoff decomposition ──────────► T08 effect/link analysis
T06 package projection decomposition ──────────► T09 boundary projector
T03 ───────────────────────────────────────────► T08
T03 + T06 + T07 + T08 ────────────────────────► T09
T04 + T05 + T09 ──────────────────────────────► T10 package emission integration
T10 ──────────────────────────────────────────► T11 package-test support
T12 integration coordinator starts before T00…T11
  C0 ─merge batch 1→ C1 ─merge batch 2→ … ─merge T11→ final gate
                                                        │
                                                        ▼
                                              A01 independent acceptance
```

可并行批次：

1. 第一批：T00、T02、T05、T06。
2. 第二批：T01。
3. 第三批：T03、T07。
4. 第四批：T04、T08。
5. 第五批：T09。
6. 第六批：T10。
7. 第七批：T11。
8. T12完成最终gate，随后A01验收。

T00/T01是架构任务；T02/T05/T06/T07是直接被本功能触碰到的营地清理，不是可省略的美化。
如果这些拆分证明无法保持行为等价，集成 Agent 不得绕过它们，必须把具体阻塞升级给用户。

T12不是“所有任务完成后才启动”的终点节点。它在C0创建integration branch，此后每批只由它
合并并发布C1…C7 checkpoint；下一批任务从对应checkpoint建branch。T12在T11合并、阶段gate和
证据提交完成后才变为completed，A01才可开始。

## 5. 任务索引

| ID | 任务 | 依赖 | Agent 输出 |
| --- | --- | --- | --- |
| T00 | [Service contract surface](tasks/P1-T00-service-contract-surface.md) | 无 | canonical 文档提交 |
| T01 | [Boundary / Recoverable 契约](tasks/P1-T01-boundary-recoverable-contract.md) | T00 | canonical 文档提交 |
| T02 | [Artifact identity 拆分](tasks/P1-T02-artifact-identity-decomposition.md) | 无 | 行为等价代码提交 |
| T03 | [Package code artifact 契约](tasks/P1-T03-artifact-code-contract.md) | T00, T01, T02 | typed DTO/identity 提交 |
| T04 | [Service requirement 单一输入 owner](tasks/P1-T04-service-requirement-input.md) | T00, T01, T03 | manifest/resolver 提交 |
| T05 | [Projection handoff 拆分](tasks/P1-T05-projection-handoff-decomposition.md) | 无 | 行为等价代码提交 |
| T06 | [Package projection 拆分](tasks/P1-T06-package-projection-decomposition.md) | 无 | 行为等价代码提交 |
| T07 | [Boundary projection 拆分](tasks/P1-T07-boundary-projection-decomposition.md) | T01 | 行为等价代码提交 |
| T08 | [Effect / link analysis](tasks/P1-T08-effect-link-analysis.md) | T03, T05 | typed analysis 提交 |
| T09 | [Boundary projector](tasks/P1-T09-boundary-projector.md) | T03, T06, T07, T08 | projection 提交 |
| T10 | [Package emission 集成](tasks/P1-T10-package-emission-integration.md) | T04, T05, T09 | compiler production-path 提交 |
| T11 | [Package test 支持](tasks/P1-T11-package-test-support.md) | T10 | test runtime/runner 提交 |
| T12 | [阶段集成协调](tasks/P1-T12-phase-integration.md) | 启动无依赖；完成需 T00–T11 | checkpoints + gate 提交 |
| A01 | [独立阶段验收](tasks/P1-A01-phase-acceptance.md) | T12 | 只读 PASS/FAIL 报告 |

## 6. Ownership 与冲突控制

- T02 独占 `artifact-identity` 的模块拆分；T03 必须建立在其提交之上。
- T05 独占 compiled → projection typed handoff 的拆分；T08/T10 只使用拆出的 owner。
- T06 独占 package artifact projection 的机械拆分；T09 不重新合并回单文件。
- T07 独占 boundary/recoverable/publication ABI projection 的机械拆分；T09 在新模块上实现即时
  boundary projection。
- T04 独占 service requirement manifest 解析与 artifact-root resolution；T10 不写第二套 parser。
- T12独占integration worktree和checkpoint；任务Agent不得自行合并前置分支。T12只能修复集成
  错误、fixture shape和遗漏调用点。若需要新增语义，应退回对应任务，
  不能以“集成修复”吞并新设计。

## 7. 阶段 Gate

T12 至少运行：

```bash
cargo test --no-fail-fast \
  -p skiff-artifact-model \
  -p skiff-artifact-identity \
  -p skiff-compiler-input \
  -p skiff-compiler-compiled \
  -p skiff-compiler-projection-input \
  -p skiff-compiler-projection \
  -p skiff-compiler-publication-abi
node scripts/verify.mjs --only compiler
cargo test --no-fail-fast -p skiff-runtime-package-test -p skiff-test-runner
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

若 `verify --only compiler` 已完整覆盖前一条中的部分 crate，T12 可以在日志中注明重叠，但不应
删除 artifact-model、artifact-identity 和 package-test/test-runner 的独立 gate。本阶段不跑 live
service、router、runtime instance 或 chat smoke。

## 8. 阶段验收样例

阶段至少留下下列可读的 source fixture 或 Rust test fixture：

1. package 声明 service dependency，编译后得到 typed requirement 和稳定 protocol expectation。
2. pure data callable 同时拥有 Local Code ABI 和 `Available` boundary contract。
3. `input.name = "new"` 形式的 mutable helper 可作为 package local API，但 boundary 为
   `Unavailable(CallerReachableMutation)`。
4. 返回 caller 参数 alias 的函数被保守标记为 boundary unavailable。
5. request-scope `any I`/native handle 走 callback capability plan；把同一 plan 放入 recoverable
   lane 时 fail closed。
6. service requirement 缺 artifact root、版本不符、protocol 不符或 alias 重复时给结构化错误。
7. 修改 diagnostic 文本不改变 identity；修改稳定 boundary contract 会改变 boundary identity。

## 9. 停止与升级条件

任何 Agent 遇到以下情况必须停止当前 feature 工作：

- canonical 文档无法唯一确定 callback capability 的 lifetime/owner 或 recoverable 的未来有效性；
- effect fact 只能靠 linker 重读 AST、解析 diagnostic 文本或猜 artifact JSON 得到；
- package 和 service 需要复制同一解析、projection 或 identity 规则；
- 必须对 `compiler/lowering/src/function_lowering.rs` 做职责性扩张而无法复用已有 typed/lowered
  facts；
- 新字段需要依赖 provider package id/build id，或把 service deployment identity 混入 code ABI；
- 阶段结束必须保留两个生产 reader/writer 才能工作。

能由已冻结架构唯一推导的，新增独立前置任务并更新 DAG；不能唯一推导的，询问用户。
