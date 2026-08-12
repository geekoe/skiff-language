# 大型重构的滚动细化与多 Agent 实施原则

> Status: reusable implementation-process reference
>
> Scope: 跨 crate、跨进程、跨持久化格式、涉及 execution ownership 或需要多 Agent 并行的
> 大型任务。本文不定义任何产品或语言语义。

大型任务不存在一套可以机械套用的完整工作流。不同任务的风险边界、真实验证路径、可并行面和
cutover 方式都不同。通用流程只能提供安全内核；每个大型任务必须在自己的 implementation 目录中
建立项目计划，并按真实依赖滚动细化 Phase。

本文沉淀通用安全内核、计划层级、任务划分、Agent 角色、worktree 使用和候选验收规则。具体项目可
增加更强门禁，但若要放宽本文的隔离或验收约束，必须在项目计划中写明理由、风险和替代证明。

## 1. 先区分五个维度

大型任务最容易出错的地方，是把以下概念混成一件事：

| 维度 | 回答的问题 | 典型载体 |
| --- | --- | --- |
| 项目 / initiative | 最终要收敛到什么系统，分几步到达 | 总体实施文档 |
| Phase | 本轮只改变哪一段支持面，进入和退出条件是什么 | Phase 文档与 result |
| Semantic Closure | 哪一个完整语义协议必须被闭合 | task DAG 中的 kernel / lane |
| Write owner | 谁可以修改哪些文件，如何避免互相覆盖 | task write set |
| Worktree | 某批提交在哪个版本空间中隔离、合流和冻结 | leaf / integration / gate worktree |

它们没有一一对应关系。crate 适合定义写锁，通常不适合定义任务；worktree 适合定义版本隔离，也不应
机械地和每个任务一一绑定。

## 2. 三层计划与决定时点

采用滚动细化，不在项目开始时冻结所有未来接口。

### 2.1 项目计划：开始实施前决定

项目计划必须先冻结：

- 当前问题、目标状态和明确非目标；
- canonical semantic authority；
- 顶层 owner / authority 分配；
- 初步 Phase DAG 及其不可逆依赖；
- 第一条可执行支持面；
- destructive retirement / publication 的最终条件；
- 每个可执行 Phase 都必须拥有的验证类型；
- 当前必须 containment 或 fail-closed 的能力。

项目计划不应提前冻结：

- 所有未来 Rust API；
- 所有 Phase 的文件级任务；
- 尚未由真实 producer 和 consumer 共同验证的 DTO；
- 为了“以后可能需要”而设计的 generic facade、兼容层或第二套 authority。

### 2.2 Phase 计划：该 Phase 的实现 Agent 启动前决定

Phase 计划必须冻结：

- exact baseline commit / tree；
- 本 Phase 的目标、非目标和支持面变化；
- producer -> transport -> consumer 的完整垂直路径；
- ownership、outcome、failure、Pending、cancel、drop 等状态转移；
- central kernel 和跨 owner API；
- Semantic Closure 列表与 task DAG；
- 必须保持的角色分离、write-set 边界和 worktree 隔离约束；
- Test Design Specification、垂直闭环证明及 focused / negative / structural evidence；
- proof harness 和唯一 canonical gate command；
- frozen candidate 与独立验收方法；
- blocker、停止条件和 evidence epoch 规则。

Phase 设计未通过独立审查，不启动 production implementation。

Phase 计划不预先冻结实际 Agent、branch、worktree 数量、路径或派发顺序。这些取决于执行时的 repo state、
ready frontier、可用 Agent 和已发现冲突，由 Execution Map 滚动决定。

### 2.3 Execution Map：每次派发前滚动决定

派发本 Phase 的任何子任务前，必须先建立 Execution Map。初始版本只细化当前已经 ready 的任务；审计、
设计审查、合流或 Problem Signal 产生新事实后，必须先更新 Map，再派发下一批任务。

Execution Map 至少记录：

- exact baseline 和当前已合流 commit；
- task ID、依赖、ready/blocked 状态和当前派发批次；
- 实际 Agent、branch、worktree、write set 和只读范围；
- 输入 commit、预期交付 commit、验证责任和合流顺序；
- conditional task 的启用条件；
- 每次调整的原因、影响范围和是否使 candidate/evidence epoch 失效。

Execution Map 是可修改的 operational artifact，不是新的架构 authority。Agent 更换、串并行调整、worktree
增减、leaf 拆并和合流顺序变化可以直接更新 Map；若调整会改变 support surface、Semantic Closure、semantic
authority、中央接口、VCP 或 Gate，则 Map 无权决定，必须回到 Phase design 并重新审查。

### 2.4 Task 任务书：派发 Agent 前决定

每份任务书必须引用 Phase 计划，并写明：

- exact baseline 或已合流依赖 commit；
- 单一目标和所属 Semantic Closure；
- 精确 write set；
- 可只读访问但不可修改的相关 owner；
- 输入接口、输出接口和禁止重新推导的事实；
- focused evidence；
- 明确禁止的 fallback、bypass、兼容猜测和范围扩大；
- 何种信号必须暂停并上报；
- 交付 commit、报告和未闭合事项。

Task 不能包含尚未决定的架构选择。执行中发现需要新选择，任务立即回到 Phase 设计层。

## 3. Semantic Closure，而不是 crate completion

任务首先按完整语义协议划分，再在协议内部按写界划分 Agent。一个 Semantic Closure 至少包含：

1. 唯一 invariant；
2. fact 的唯一 producer；
3. fact 的完整 transport；
4. 最终 consumer；
5. success / error / Pending / cancel / unwind / drop 路径；
6. unsupported 路径的 fail-closed 位置；
7. 一个经过真实组合边界的可观察证明。

例如 lifecycle 不是三个互不相关的“compiler 完成”“VM 完成”“heap 完成”，而是一条闭环：

```text
source-owned lifecycle fact
  -> lowering / artifact transport
  -> exact link resolution
  -> VM lifecycle executor
  -> heap physical transition
  -> overwrite / return / tail / unwind / drop
```

若为了满足“一 Agent 一 crate”而把一个状态机拆给多个 Agent 分别解释，crate 隔离反而会制造多重
authority。中央状态机、原子 ownership transfer 或 outcome arbitration 可以跨 crate，但只能有一个
kernel owner；其余 Agent 通过已审查的窄接口实现 leaf。

## 4. 每个 Phase 的标准生命周期

Phase 内容不同，但生命周期固定：

```text
M0  initial Execution Map for the ready frontier
  -> B0  baseline audit
  -> D0  phase design
  -> R0  independent design review
  -> M*  refresh Execution Map before each dispatch wave
  -> T0  executable proof harness
  -> K0  semantic kernel
  -> L*  parallel leaves + E* evidence
  -> I0  merged integration proof
  -> F0  freeze exact candidate
  -> G0  executable Phase Gate on the frozen candidate
  -> A0  independent read-only acceptance verdict
  -> H0  result and next-phase handoff
```

### 4.1 Baseline audit

只读取证，确认真实生产路径、现有 owner、已启用能力、masking relationship 和当前验证入口。调查 Agent
不修改代码，不需要 worktree。审计结果必须区分：

- 当前可触发的错误语义；
- 接通后会出错的结构阻断；
- 明确 fail-closed 的完成缺口；
- 继承自旧系统、但因本项目支持面扩大而必须重新审计的假设。

### 4.2 Phase design 与独立设计审查

一个 design owner 统一写 Phase 设计。另一个未参与设计的 read-only reviewer 至少检查：

- API 是否能表达完整 ownership 和 replacement result；
- 同一事实是否有多个 owner 或多种表示；
- 下游是否被迫从字符串、类型外形或 opcode 重新推导事实；
- Ready / Pending / error / cancel / drop 是否闭合；
- unsupported 是否在唯一边界稳定拒绝；
- 验收是否只覆盖 happy path；
- implementation / retirement 是否被错误地放在同一 Phase。

reviewer 给出 `PASS` 或带 blocker 的 `FAIL`。不能以“实现时再看”通过设计缺口。

### 4.3 Test design、kernel、leaf 和 evidence

任务类型使用以下词汇：

| 类型 | 职责 |
| --- | --- |
| `K` Kernel | 中央状态机、原子协议和共享语义接口 |
| `L` Leaf | producer、transport、consumer 或 adapter 的非重叠实现 |
| `T` Test design / proof harness | scenario matrix、fixture、failure injection、观测和 canonical command |
| `E` Evidence | 执行 proof harness、negative matrix、结构检查并保存 candidate-specific evidence |
| `I` Integration | 串行合流、中央 wiring、merged-state proof |
| `A` Acceptance | 对 frozen candidate 的独立只读判定 |

Test design 不是实现完成后的补测。Phase design 必须先从 invariant、状态转移和支持面设计 scenario
matrix；设计审查通过后，test owner 优先把它落成可执行的 red/green harness。Leaf 实现可以与剩余测试
编码并行，但不能早于 proof harness 的入口、终点和断言设计。

Evidence 不是最后补的装饰。它是 exact candidate 执行 proof harness 后产生的结果。测试不能使用
test-only execution API 绕过 production composition。

### 4.4 Integration、freeze 和 acceptance

唯一 integrator 按 DAG 合流 commit，负责中央 wiring 和 merged-state proof。integrator 不得为了使分支
汇合而临时加入：

- 宽松 type equivalence；
- package / symbol / binding 字符串特判；
- registry mismatch bypass；
- 缺 fact 后的默认值；
- 第二套执行路径；
- 没有 owner 的 sidecar registry。

出现上述需求，candidate 退回设计或原 Semantic Closure owner。

合流完成后冻结 exact commit 和 tree。冻结后任何代码、fixture、harness、binary 或 artifact identity
变化都会开始新的 evidence epoch，旧验收不得拼接复用。最终 acceptance owner 不参与本 Phase 的生产
实现，只读审查同一 frozen candidate。

## 5. 垂直闭环证明（Vertical Closure Proof，VCP）

### 5.1 为什么不用笼统的“端到端”

“端到端测试”常被理解成必须启动整个产品，也常被缩减成只看最终响应。两种理解都不适合作为每个
Phase 的反馈机制。我们需要的是一个范围随 Phase 改变、但能证明完整责任链的概念。

本文将它命名为 **垂直闭环证明（Vertical Closure Proof，VCP）**：

> 从本 Phase 的真实事实生产者或公开输入开始，经过本 Phase 声称完成的所有 production-shaped
> 组合边界，到达最终消费者，并同时证明外部结果和关键内部 ownership / route / state 事实。

VCP 可以比 whole-product E2E 小，但不能是单 crate 测试，也不能跳过中间层。

### 5.2 VCP 的强制属性

每个可执行 Phase 的 VCP 必须：

1. **Vertical**：跨过本 Phase 改动的所有 producer / transport / consumer owner；
2. **Production-shaped**：调用真实 production composition 和 admission API；
3. **Phase-scoped**：只证明本 Phase 的支持声明，不假装覆盖未来能力；
4. **Observable**：同时断言外部结果、exact route/owner 和关键状态转换；
5. **Diagnostic**：失败可以定位在哪个边界，不只得到“响应错误”；
6. **Negative-capable**：至少一个 malformed / unsupported / losing-race 情形稳定 fail closed；
7. **Deterministic where possible**：优先使用可控 clock、store、host completion 和动态端口，不依赖 LLM
   随机行为充当语义证明。

允许注入测试 store、clock、网络 peer 或 fake host completion；不允许注入手工构造的 verified image、
linked target、VM fiber、内部 owner token 或绕过 production loader/linker/scheduler 的 test-only executor。
测试专用观测点可以存在，测试专用执行路径不可以存在。

### 5.3 何时必须拥有 VCP

- 项目计划先定义每个 Phase 的 VCP 终点；
- Phase 设计必须给出精确入口、fixture、观测事实和命令 owner；
- 第一行 production implementation 开始前，VCP harness 必须已经存在，或作为该 Phase 的先行
  validation-infrastructure task 被接受；
- 实现过程中，integrator 在每个 kernel / lane join 后运行最小 VCP；
- frozen candidate 上由 acceptance owner 重跑完整 VCP。

若找不到可行 VCP，不允许先实现、最后集成。Phase 标记 `blocked`，先讨论：

1. 当前是否缺少真实 composition seam；
2. 是否需要一个 in-process production composition harness；
3. 是否必须启动隔离进程才能观察最终消费者；
4. 需要增加的是只读 observability，还是会形成新的执行 API；
5. 本 Phase 是否太大，应该再拆分。

## 6. Test design、VCP 和门禁的关系

三者是不同层次：

```text
canonical semantics / invariant
  -> Test Design Specification
  -> executable tests + fixtures + scripts (proof harness)
  -> one VCP run and its evidence manifest
  -> Phase Gate evaluates all required evidence on one frozen candidate
  -> independent acceptance records the verdict
```

- **Test design** 决定要证明什么，以及场景是否覆盖本 Phase 声明的全部语义维度；
- **VCP specification** 定义跨哪些 owner、使用什么 fixture、断言哪些外部和内部事实；
- **VCP harness** 是测试、fixture、failure injection、观测点和脚本的可执行实现；
- **VCP evidence** 是 harness 在一个 exact candidate 上运行后产生的 manifest；
- **Gate** 是决策规则：只有 VCP、focused、negative、structural 和本 Phase 其它必要证据都通过，才返回
  PASS；
- **Acceptance** 是独立 owner 对 frozen candidate 执行 gate、核对证据并作出的阶段结论。

因此 VCP evidence 是门禁的必要组成部分，但不是整个门禁。一个 VCP 成功不能掩盖 ownership negative、race、
结构旁路或性能上限失败；一堆 crate test 成功也不能替代 VCP。

更精确地说，**VCP 是 proof obligation，test/script 是 proof carrier，Gate 是 decision function**。同一个
测试二进制可以同时承载 focused case 和 VCP scenario，但 result 中必须分别记账；不能因为它们由同一命令
启动，就把三种职责重新混成“测试过了”。

### 6.1 Test Design Specification

每个 Phase 必须在实现任务派发前建立 coverage matrix。它至少把以下内容一一映射：

| Semantic item | Scenario | Test level | Fixture / injection | Observable assertion | Failure owner |
| --- | --- | --- | --- | --- | --- |
| invariant / transition | success, boundary or losing branch | unit/contract/VCP/structural | exact input | result + state/owner fact | task ID |

测试覆盖以**声明支持面的语义维度和状态转移**为准，不以行覆盖率或测试数量代替。适用时至少设计：

- ordinary success 与 boundary values；
- 每个状态机 transition；
- malformed / unsupported / identity mismatch；
- error、cancel、deadline、unwind 和 drop；
- complete-before-register、duplicate/late completion 等竞争分支；
- resource/memory/fuel 上限；
- exact owner/route 和禁止 fallback；
- 重启、session disconnect 或 publication failure 等生命周期事件。

不适用的维度必须写明理由，不能因当前实现没有相应分支而从 matrix 消失。

### 6.2 可执行载体

VCP 和 gate 都必须以仓库内可执行产物出现，而不只是 result 文档中的手工描述：

1. canonical fixture / corpus；
2. production-shaped test harness；
3. schema-validated evidence manifest；
4. 一个注册在仓库验证图中的 canonical command / selector；
5. gate checker，验证 exact commit/tree、非零场景、无 skip、manifest 完整和所有子证明状态；
6. 必要的只读 observability；
7. 可重复的 failure injection。

测试实现可以分布在多个 crate/source fixture，门禁入口必须唯一。不得要求验收者从 README 复制多条命令
再人工拼出 PASS。不可机械验证的架构审查项可以保留人工 verdict，但必须枚举、记录 reviewer 和证据；
不能用笼统的“人工确认”代替可执行检查。

### 6.3 Gate 分层

一个 Phase Gate 通常聚合：

| Evidence class | 证明内容 |
| --- | --- |
| focused/unit | 局部算法和边界值 |
| producer-consumer contract | 跨 crate 事实没有丢失或重建 |
| VCP | production-shaped 垂直责任链闭合 |
| negative/race/lifecycle | 失败、竞争、terminal 和资源路径 |
| structural/reverse search | 没有第二 authority、fallback、bypass 或旧路径 |
| budget/performance | 本 Phase 声明的有界性 |
| subject/system regression | 与已接受能力组合后没有回归 |

Phase plan 必须声明哪些 evidence class 必选以及为什么。Gate runner 必须 fail closed：缺 manifest、零测试、
skip、命令未运行、candidate identity 不同或证据属于旧 epoch 都是 FAIL。

### 6.4 测试资产的累计和唯一 ownership

测试按 semantic behavior 组织和记账，不按实现 crate 被动堆积。每个 scenario 在 coverage ledger 中只有一个
canonical owner；其它层可以有必要的 focused case，但不能用多份近似断言制造“覆盖很多”的错觉。

一个 Phase accepted 后：

- 该 Phase 的 VCP harness、negative matrix 和 manifest checker 成为后续 Phase 的永久 regression asset；
- 后续 Gate 必须组合运行所有仍受影响的 accepted scenarios；
- 修改 fixture、断言、observability 或 gate checker 会使相关 evidence epoch 失效；
- 删除或降级 scenario 必须由 canonical semantics / support-surface 变化解释；
- 新发现的未覆盖 transition 先进入 coverage ledger，再决定归属当前 blocker 或后续明确 Phase；
- test owner 负责减少重复 fixture 和 test-only model，不把生产模型复制到测试里重新实现。

这样 coverage 是随支持面单调积累的语义账本，而不是每个 crate 各自维护、最终无人能说明是否完整的测试
数量集合。

## 7. Worktree 规则

Worktree 按并发写入和版本隔离创建，而不是一任务一个。

### 7.1 拓扑约束

具体 worktree 由当前 Execution Map 创建和记录。下面是允许的典型形状，不是每个 Phase 必须预建的固定
清单：

```text
main checkout                       始终在 main，保持可构建、可运行
<project>-pN-integration worktree   本 Phase 唯一合流线
<project>-pN-<leaf> worktree        每个并发 write owner 一个
<project>-pN-gate worktree          frozen candidate，只读验收
```

- 只读调查不需要每个 Agent 各建 worktree；若 main 不能在调查窗口保持 exact baseline，Execution Map 应
  分配一个共享的 detached baseline worktree；
- 同一 Agent 串行完成的几个小 leaf 可复用一个 leaf worktree；
- 多个 Agent 并发写代码时不得共享 worktree；
- 修改同一中央文件的任务不并行，归一个 kernel / integrator owner；
- worktree 直接建立在 workspace 容器目录下；
- Phase 接受并合入 main 后删除 Phase worktree，下一 Phase 从新的 exact main 建立；
- gate worktree 从 frozen commit 创建，验收过程中不得接受生产代码写入。

### 7.2 Main 的约束

Main 可以接收完整的文档计划和已接受的 Phase，不接收：

- interface-only placeholder；
- 不能编译的中间状态；
- `Checkpoint:` 式跨层临时拼接；
- 需要下一个未接受 commit 才恢复语义的半协议；
- 未经 frozen-candidate acceptance 的 destructive retirement。

Worker 的恢复点 commit 留在 leaf branch；它是可恢复性机制，不是 accepted contract。

## 8. Agent 安排与职责分离

Agent 数量由独立 write owner 和 evidence owner 数量决定，不以“尽可能多并行”为目标。

推荐角色：

- **Investigator**：只读取证；
- **Design owner**：统一 Phase 设计；
- **Test design owner**：从 canonical invariant 建立 coverage matrix 和 proof-harness contract；
- **Design reviewer**：独立审查设计，不写本 Phase 生产实现；
- **Kernel owner**：实现中央协议，可拥有因原子性必须一起修改的跨 crate write set；
- **Leaf owner**：实现一个非重叠 producer / transport / consumer lane；
- **Evidence owner**：实现/运行已接受的测试设计并保存 candidate-specific evidence；
- **Integrator**：唯一合流和中央 wiring owner；
- **Acceptance owner**：冻结后独立只读验收。

角色可以跨 Phase 复用，但同一 Phase 至少保持以下分离：

- 最终 acceptance owner 不参与 production implementation；
- 多个 Agent 不共同拥有同一状态机；
- leaf owner 不自行扩大 central API；
- integrator 不通过兼容猜测解决语义冲突。

资源不足时可以减少 leaf 数量、串行实现；不能取消独立 acceptance 或把互相冲突的 authority 交给多个
Agent。

## 9. 支持面、状态和门禁

能力状态统一使用：

| 状态 | 含义 |
| --- | --- |
| `accepted` | 在 frozen candidate 上有 VCP 和独立验收 |
| `enabled-unaccepted` | 当前可达但没有足够语义证明；必须优先 containment 或修复 |
| `disabled` | 在唯一边界明确 fail closed |
| `planned` | 尚未实现且不应可达 |

“代码存在”“接口存在”“crate test 通过”都不能把能力标成 `accepted`。

Phase 状态建议使用：

```text
planned -> design-ready -> implementation-ready -> integrated -> frozen -> accepted
                                  \-> blocked <-----------------------/
```

只有 `accepted` Phase 可以解锁下一 Phase 的 production implementation。后续 Phase 可以提前进行只读
调查，但不得在未接受接口上扩张执行面。

## 10. Retirement、cutover 和兼容规则

Cutover / deletion Phase 只能切换、删除和证明不可达，不能首次实现新语义。执行 destructive retirement
前必须具备：

- replacement 的完整支持面清单；
- replacement frozen candidate 的独立验收；
- reference-derived behavioral evidence；
- rollback 到旧 binary / commit 的操作方案；
- reverse-search 和 production route proof。

未发布项目可以删除旧格式，但“不需要历史兼容”不等于可以跳过行为闭环。兼容 fallback、双 authority
和 permissive equivalence 不能代替迁移计划。

若旧路径已提前删除，后续计划不再虚构一次 cutover；应改为逐能力重新 admission，最终 Phase 只做
whole-system closure。

## 11. 反模式清单

以下现象默认是 blocker，而不是正常集成成本：

- 先冻结接口外形，producer / consumer 尚未共同演算完整协议；
- interface stub 合入 main 后允许 workspace 长期不编译；
- 只看 direct dependency，不等待上游 semantic acceptance；
- 为了提高并行度拆开必须原子拥有的状态机；
- 多个 Agent 直接写同一 checkout；
- 一个 checkpoint 同时修改 compiler、linker、VM、host 来“接起来”；
- 合流时增加 type equivalence、字符串 dispatch、registry bypass 或默认 owner；
- verifier / validator 重建上游缺失事实；
- 只有 unit / crate test，真实 composition 到最后才首次执行；
- 实现完成后才开始设计测试，由各 leaf 被动补琐碎 happy-path case；
- VCP 只存在于文档，没有 fixture、canonical command 或 manifest；
- gate 依赖人工拼接多次运行，允许 skip、零测试或跨 candidate 复用 evidence；
- replacement 未接受就删除旧行为 oracle 和大批场景测试；
- 用测试专用 executor、手写 linked image 或 fake seal 证明 production path；
- 把当前明确错误的可达路径列为后续优化。

## 12. 项目计划最低模板

每个大型项目至少创建：

```text
doc/implementation/<project>/
  README.md                         # 总目标、authority、Phase DAG、状态
  <process-principles-or-link>.md   # 所采用的过程规则
  phases/
    phase-0-....md                  # 当前已细化 Phase
    ...
  tasks/                            # Phase 通过设计审查后再创建
  results/                          # frozen candidate evidence / verdict
```

每个 Phase result 至少记录：exact commit/tree、worktree/branch、task commits、VCP 命令和 evidence、
negative/structural proof、独立 acceptance verdict、未接受能力、下一 Phase 输入。

本文的目的不是增加文档数量，而是让实现者在写第一行跨层代码之前能够回答：**谁拥有事实、协议如何
闭环、如何在本 Phase 当场证明它，而不是等最终集成才发现。**
