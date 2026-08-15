# 大型任务的双线并行与滚动细化原则

> 流程步骤的唯一权威是 [`runbook.md`](./runbook.md) 的 9 步。本文是**语义原则参考**（authority、Semantic
> Closure、VCP 判据、反模式），不另立流程步骤；与 runbook 冲突时以 runbook 为准。

> Status: reusable implementation-process reference
>
> Scope: 跨 crate、跨进程、跨持久化格式、涉及 execution ownership 或需要多 Agent 并行的大型任务。
> 本文不定义任何产品或语言语义。

大型任务没有一套可以机械套用的完整工作流。通用流程只固定一个小的安全内核：同一份 Phase Contract、
一条开发线、一条证明线、滚动 Execution Map，以及 frozen candidate 上的独立验收。调查和设计由实际问题
条件触发；architecture/decision 文档不设置独立 review/PASS 前置门禁。专项 review 只针对已经形成的实现候选，
不是开码前的文档流水线。

本文的目标不是增加角色和文档，而是让工作尽快产生可执行反馈，同时避免开发者自己定义成功、自己制造
证据、自己宣布通过。

## 1. 五个不同维度

| 维度 | 回答的问题 | 典型载体 |
| --- | --- | --- |
| 项目 / initiative | 最终收敛到什么系统，分几步到达 | 总体实施文档 |
| Phase | 本轮只改变哪一段支持面，如何算完成 | Phase Contract 与 result |
| Semantic Closure | 哪一个完整语义协议必须闭合 | 开发线中的 kernel / lane |
| Write owner | 谁可以修改哪些文件，如何避免覆盖 | Execution Map 的 write set |
| Worktree | 某批提交在哪个版本空间中隔离和合流 | leaf / integration / gate worktree |

它们没有一一对应关系。crate 可以帮助定义写锁，通常不能定义任务；worktree 用于版本隔离，也不应机械地
和每个任务一一绑定。

## 2. Phase Contract：两条线的共同前提

每个可执行 Phase 在派发生产代码或证明代码前，必须有一份精简、无内部矛盾的 Phase Contract。它只冻结：

- exact baseline commit/tree 和状态为`accepted`的上游result；旧receipt或`candidate-pass`不能单独充当baseline；
- 本 Phase 的外部目标、accepted surface 和明确非目标；
- 不得破坏的 authority、ownership、failure 和 lifecycle invariant；
- 一个最小 VCP 的公开输入、production-shaped 边界和预期外部结果；
- unsupported 能力的 fail-closed 要求；
- 完成标准、停止条件和需要用户决定的边界。

Phase Contract 不默认冻结：

- 所有未来 Rust API、类型名和文件级任务；
- 完整测试矩阵、每条命令和所有 failure injection；
- 实际 Agent、worktree 数量和派发顺序；
- 单个开发 lane 内部、私有且容易撤销的实现选择；
- 尚未由真实 producer/consumer 共同验证的 facade、兼容层或第二 authority。

Phase Contract 可以来自 canonical architecture、用户决定和前一 Phase 的 accepted handoff。只有当这些输入
不能唯一给出共享目标时，才启动 §4.2 的 Design task。不能为了形式完整而先写一份包揽全部未来细节的设计书。

## 3. 核心执行模型：开发线与证明线

每个实现 Phase 只有两条稳定工作线：

```text
                         Phase Contract
                      /                  \
        Development Line                 Proof Line
        production implementation        tests / VCP / Gate / evidence
                      \                  /
                   rolling integration proof
                              |
                       frozen candidate
                       /              \
             semantic review    independent Acceptance
                       \              /
                         accepted result
```

### 3.1 Development Line

开发线负责让 production system 满足 Phase Contract：

- central kernel、owner state machine 和原子协议；
- producer、transport、consumer 与 boundary leaf；
- capability containment 和旧路径 retirement；
- 必要的只读 observability；
- 开发者自己的 focused/unit tests。

开发 Agent 可以进行自己 write set 内的局部设计，也可以实现自己提出的共享设计决定。它不能修改 Proof Line
的通过标准来迁就实现，也不能给 frozen candidate 作最终验收。

### 3.2 Proof Line

证明线从 Phase 开始就与开发线并行，负责把 Phase Contract 变成独立、可执行的反证和通过条件：

- expected-red contract/negative tests；
- production-shaped VCP fixture 和 harness；
- Gate selector、aggregator、manifest/checker 和自测；
- candidate-specific raw evidence；
- accepted capability 的 regression 组合。

Proof Agent 不修改 production implementation 使测试通过，不用 test-only executor、手写 linked image、fake seal
或硬编码 PASS 代替真实路径。开发线可以提供窄的 production observability，但 observation contract 由两条线
共同消费，不能成为第二 execution authority。

### 3.3 Acceptance 是证明线的末端，只在freeze后执行

测试设计、harness 和 Gate 实现可以与 production code 并行；最终 Acceptance 只能在合流并冻结 candidate 后
执行。Acceptance Agent 必须是没有写本 candidate production/test/Gate 的全新只读 owner。它运行 canonical
Gate、核对 raw evidence 和 exact commit/tree，然后给出 `PASS` 或 `FAIL`。Frozen-candidate semantic review与
Acceptance可以在同一freeze后由不同只读owner并行；范围较大时semantic review也可按互斥主题并行读取，最后一次
合并findings，避免一个问题一个问题串行发现。任一修复都会产生新candidate/epoch，两边旧verdict都不可复用。

因此“开发线 + 测试验收线”指两个持续 workstream，不表示测试作者同时拥有最终验收 verdict。

## 4. 条件支持任务

Clarification、Design 和专项 Review 是按问题派发的临时任务，不是固定 Phase 角色，也不构成默认全局 barrier。

### 4.1 Clarification Agent（澄清 Agent）

Clarification 回答“当前事实是什么”，不决定“目标应该是什么”。只有同时满足以下条件才单独派发：

```text
存在一个明确事实问题
AND 当前权威资料或已有证据不能回答
AND 答案影响一个正在进行的任务
AND 单独调查比多个 owner 重复调查更划算
```

典型问题包括：真实 production 入口、当前调用方、某路径是否可达、现有 Gate 是否保存 raw evidence。

每个 Clarification task 必须写明：

- 一个具体问题和 exact baseline；
- 消费答案的 task；
- read scope、`status_after` 和停止条件；
- 所需证据形式；
- 明确禁止目标 API、迁移顺序、Phase owner 或 acceptance verdict。

默认交付是简短 handoff 加文件/符号/commit 证据，不要求仓库内长报告。只有会被多个任务或后续 Phase 稳定
复用的事实才形成文档。Clarification 只阻塞依赖该答案的 task；其它 ready task 继续。

当前 owner 能在自己 task 的正常代码阅读中回答的问题，不另派 Clarification Agent。

### 4.2 Design Agent（设计 Agent）

Design 回答“多个合理目标中选择哪一个”。主 Agent 仅在下式成立时创建 Design task：

```text
NeedDesign =
  存在尚未决定的目标选择
  AND
  (
    影响两个以上 write owner
    OR 修改 Development Line 与 Proof Line 的共同合同
    OR 改变 authority / ownership / failure / lifecycle semantics
    OR 改变公共类型、artifact/schema、持久状态或跨模块边界
    OR 选错后需要大范围返工且难以局部撤销
  )
```

以下情况不启动 Design Agent：

- 不清楚当前事实：走 Clarification；
- 单一 write owner 内部、私有、可逆的实现选择：开发 Agent 决定；
- scenario、assertion 和 Gate 载体选择：Proof Line 决定；
- Agent、worktree、并发和合流顺序：主 Agent通过 Execution Map 决定。

一个 Design task 只关闭一个决定或一个不可分割的决定簇，交付包含：decision、理由、被拒方案、Contract/API
影响、消费者、证明义务和未决项。它不顺手生成完整 Phase DAG、所有测试场景或上千行迁移说明。

设计者仍然是球员，可以随后进入开发线实现该决定。设计文档不需要交给另一个 Agent 审查后才能开码；最终
frozen implementation candidate 仍由未参与候选写入的 semantic reviewer / Acceptance owner核对实际代码和证据。

### 4.3 Architecture / Design 文档不设 review gate

Design task 写到足以指导当前实现即可，不要求为完备性遍历所有 architecture/reference 文档，也不要求独立
Design Review 或 PASS receipt。包括下列高风险共享事实也直接进入实现与可执行验证：

- authority 或 ownership owner；
- persistent/public schema 或跨进程 ABI；
- irreversible cutover / deletion；
- Ready/Pending/error/cancel/drop 等跨 owner 状态机；
- Development/Proof 两条线的共同成功定义。

这些决定必须写出核心 invariant、failure envelope、迁移边界和可执行验收条件；正确性由 focused tests、Gate、
frozen candidate semantic review 与 Acceptance 证明。实现过程中发现真实冲突时就地 amendment，外围文档措辞
漂移记为 non-blocking debt，不能扩大成开放集合的全仓文档审查。

## 5. Execution Map 与主 Agent

主 Agent 维护滚动 Execution Map、派发、监控、接管、机械合流、freeze 和 acceptance handoff。它不写最终
verdict，也不在 integration worktree 临时发明语义修复。

Execution Map 初始版本只记录当前 ready frontier：

- exact baseline/current integration commit；
- Phase Contract identity；
- Development/Proof 两条线的首批 task；
- task ID、依赖、Agent、branch/worktree、write/read set；
- `started_at`、`status_after`、expected output 和 join condition；
- conditional Clarification/Design/Review 的触发条件；
- first-code、first-proof-attempt 和 integration checkpoint；
- candidate/evidence epoch。

任一 Agent 完成、失败、中断或交付 commit 后，主 Agent立即核对 handoff、更新 Map、重算 ready frontier，派发
所有无依赖/写冲突的任务。不等待整齐 wave。

### 5.1 进度控制

并发目标是缩短关键路径，不是增加 Agent 数。每个 Phase 必须定义：

- 第一个非文档 commit 的目标检查点；
- 第一次 executable proof/VCP attempt 的目标检查点；
- 每个 task 的短 `status_after`；
- 超时后的 partial handoff、拆分或 takeover 方法。

一个实现 Phase 如果持续只有审计/设计文档而没有非文档 commit 或 executable attempt，主 Agent 必须暂停并
重排，不得用“快写完报告了”无限延长。默认 Clarification checkpoint 应短于普通开发 task；超过预计范围的
调查应返回当前证据和剩余问题，而不是扩写成完整系统设计。

同一 worktree 同时只有一个 write Agent。takeover 前先停止旧 owner；可并行派多个 read-only diagnostic task，
但中央状态机不能因为超时拆成多个 write authority。

### 5.2 Task handoff

非只读 task 交付一个可独立合流的 commit，并报告：input/output commit、实际 write set、合同 disposition、
focused commands、日志位置、未运行项、remaining risk 和下一 ready task。只读 clarification 默认交付短证据
handoff，不强制 commit。

预计超过 30 秒的命令将输出重定向到临时或 durable 文件并可轮询。Development Agent 不自行运行无关全仓
测试；Acceptance Agent 必须完整运行 Phase Contract 指定的 canonical Gate，中断等于未运行。

## 6. Semantic Closure，而不是 crate completion

开发任务按完整语义协议划分，再在协议内部按写界分配 Agent。一个 Semantic Closure 至少包含：

1. 唯一 invariant；
2. fact 的唯一 producer；
3. fact 的完整 transport；
4. 最终 consumer；
5. success/error/Pending/cancel/unwind/drop 路径；
6. unsupported 的 fail-closed 位置；
7. 一个经过真实组合边界的可观察证明。

例如 lifecycle 不是三个互不相关的 crate completion：

```text
source-owned lifecycle fact
  -> lowering / artifact transport
  -> exact link resolution
  -> VM lifecycle executor
  -> heap physical transition
  -> overwrite / return / tail / unwind / drop
```

中央状态机、原子 ownership transfer 或 outcome arbitration 可以跨 crate，但只能有一个 kernel write owner；
其余 Agent 通过已接受的窄接口实现 leaf。

## 7. VCP、测试与 Gate

### 7.1 垂直闭环证明（Vertical Closure Proof，VCP）

VCP 从本 Phase 的真实事实生产者或公开输入开始，经过本 Phase 声称完成的 production-shaped 组合边界，
到达最终消费者，并同时证明外部结果和关键 owner/route/state 事实。

VCP 可以比 whole-product E2E 小，但不能是单 crate 测试，也不能跳过中间层。它必须具备：

1. Vertical：跨过相关 producer/transport/consumer；
2. Production-shaped：调用真实 composition/admission API；
3. Phase-scoped：只证明本 Phase 支持面；
4. Observable：外部结果加关键内部事实；
5. Diagnostic：失败可定位边界；
6. Negative-capable：至少一个 malformed/unsupported/losing branch；
7. Deterministic where possible：使用可控 clock/store/peer/host completion。

允许注入测试 store、clock、网络 peer 或 fake host completion；不允许注入手工 image、linked target、
VM fiber、内部 owner token 或绕过 production loader/linker/scheduler 的 test-only executor。

Phase Contract 必须在两条线启动前定义 VCP 的入口、终点和预期结果；可执行 harness 由 Proof Line 从第一批
task 开始实现，并与 production code 并行。找不到可行 VCP 时，只阻塞依赖该 seam 的工作：先 Clarification；
若需要新增共享 execution authority，再触发 Design 和必要 review。

### 7.2 测试设计是滚动的

Proof Line 首先把 Phase Contract 转成最小 expected-red 场景，然后随 Development Line 暴露的真实状态转移
滚动补充。第一行 production code 之前不要求冻结完整矩阵，但必须已有：

- 最小外部成功结果；
- 至少一个 fail-closed companion；
- production seam 约束；
- 不得由 harness 伪造的关键事实。

覆盖按 semantic behavior 和状态转移组织，不按 crate 行覆盖率或测试数量。适用时逐步闭合 boundary、identity
mismatch、error/cancel/deadline/unwind/drop、race、resource/memory/fuel limit 和 no-fallback/no-bypass。

### 7.3 Proof carrier、evidence 和 Gate

```text
Phase Contract
  -> executable tests / fixtures / scripts
  -> VCP + focused + negative + structural evidence
  -> Gate evaluates one exact candidate
  -> independent Acceptance records verdict
```

VCP 是 proof obligation，test/script 是 proof carrier，Gate 是 decision function。Gate 必须 fail closed：缺
manifest、零场景、skip、命令未运行、dirty/stale candidate、tampered raw evidence 或跨 epoch 拼接都是 FAIL。

每个Phase Gate必须导出一个transitive `phaseNWorkloadSpecs(root)`，组合上一Phase的workload specs，而不是嵌套
旧Gate或接受旧receipt。组合必须保留每个spec的`testFormat`、lanes和已有`expectedTests`，并用candidate-owned
显式catalog记录source/parent/origin chain；不得把嵌套id前缀当作provenance authority；
本Phase所有`testFormat != null` workload声明positive `expectedTests`，继承spec若历史上缺字段则显式记录，不猜默认值。每个
`cargo test` workload带且只带一次`--no-fail-fast`；composer可以幂等插入该orchestration flag，但不能改target/
filter/harness args，build/fmt/clippy也不能带。outer runner捕获nonzero后仍执行并记录其它workload；zero/skip/
todo/ignored/cancelled和未执行项均FAIL。这样一次Gate暴露全部已到达边界的问题，而不是把E2E变成串行问题探测器。

一个 accepted Phase 的 VCP/negative/Gate assets 成为后续 Phase regression。修改 fixture、assertion、observability
或 checker 会使相关 evidence epoch 失效。

## 8. Integration、freeze 和 acceptance

唯一 integration owner 按 rolling join 合流 Development/Proof commits，运行受影响的最小 contract/VCP preflight，
并重算 ready frontier。Integrator 不通过 type equivalence、字符串特判、registry bypass、默认 owner、第二执行
路径或无 owner sidecar 解决冲突；出现这类需求退回原 owner或触发 Design task。

完成两条线的 Phase Contract obligations 后冻结 exact commit/tree。冻结后任何 production/test/fixture/Gate/
event/schema 变化都开始新 evidence epoch。

Acceptance Agent 在 detached clean worktree 执行完整 Gate，核对 raw evidence 而非只看 exit code；semantic
review可在同一frozen candidate并行进行。`FAIL`返回对应Development/Proof owner；修复后重新freeze，旧review、
verdict和receipt不可复用。只有两项均PASS，result status才可写`accepted`。

## 9. Worktree 规则

Worktree 按并发写入和版本隔离创建，不按角色或任务机械一一对应：

```text
main checkout                       始终停在 main
<project>-pN-integration            唯一合流线
<project>-pN-<write-lane>           每个并发 write owner
<project>-pN-gate                   frozen candidate，只读验收
```

- Clarification 通常共享 exact detached baseline，不为每个问题新建 worktree；
- 同一 Agent 串行的小 task 可以复用 clean leaf worktree；
- 多个 Agent 并发写代码不得共享 worktree；
- 修改同一中央状态机的任务归一个 kernel owner；
- main checkout 不切换到任务分支；
- worktree 直接建立在 workspace 容器目录；
- Phase accepted 并合流后清理 Phase worktree；
- 未提交状态不能作为下游隐式输入。

## 10. 最小角色分离

角色数量由当前 write owner 和 proof owner 决定。稳定存在的职责只有：

- **Main/Integrator**：Map、调度、监控、机械合流、freeze；
- **Development owner(s)**：production implementation；
- **Proof owner(s)**：canonical tests/VCP/Gate/evidence；
- **Acceptance owner**：frozen candidate 的独立 verdict。

Clarification 与 Design 都是 conditional task；专项 Reviewer只用于实现候选。强制分离只有：

- Frozen candidate semantic reviewer / Acceptance owner 不参与 candidate production/test/Gate 写入；
- Proof owner 不修改 production code 来制造 PASS；
- 多个 owner 不共同拥有同一状态机；
- integrator 不在 merge 时补语义。

设计者可以成为开发者；开发者可以写局部 unit tests；同一 Agent 可以串行拥有多个不冲突的 leaf。不要为了
形式上的角色纯度增加 handoff 和全局 barrier。

## 11. 支持面、状态和 retirement

能力状态统一使用：

| 状态 | 含义 |
| --- | --- |
| `accepted` | frozen candidate 上有 VCP 和独立验收 |
| `enabled-unaccepted` | 当前可达但证明不足，必须优先 containment 或修复 |
| `disabled` | 在唯一边界明确 fail closed |
| `planned` | 未实现且不应可达 |

“代码存在”“接口存在”“crate test 通过”都不能把能力标成 `accepted`。

Cutover/deletion 只能切换、删除和证明不可达，不能首次实现新语义。执行 destructive retirement 前必须有
replacement support matrix、frozen-candidate acceptance、behavioral evidence、rollback 和 reverse-search/
production-route proof。

## 12. 反模式

以下现象默认要求停止和重排：

- 把 baseline audit、完整设计和全量 review 设为所有代码之前的固定瀑布；
- 为了提高并发按子系统启动大量没有具体消费者的调查；
- Clarification 输出目标 API、迁移顺序或 Phase task；
- Design task 包揽整个 Phase 的详细实现和测试矩阵；
- 多个并行 task 最后依赖一个全量 join，尾部 Agent阻塞全部开发；
- 一个实现 Phase 长时间只有文档提交，没有非文档 commit 或 executable attempt；
- interface stub 合入 main 后允许 workspace 长期不编译；
- 为并发拆开必须原子拥有的状态机；
- 多个 Agent 直接写同一 checkout；
- 合流时增加 type equivalence、字符串 dispatch、registry bypass 或默认 owner；
- verifier/validator 重建上游缺失事实；
- Proof Line 最后才启动，真实 composition 到最终验收才首次执行；
- VCP 只存在于文档，没有 fixture、canonical command 或 raw evidence；
- Gate 依赖人工拼接命令或允许 skip/zero/stale evidence；
- 开发者自己定义场景、写实现、生成 PASS 并作最终验收；
- replacement 未接受就删除旧行为 oracle。

## 13. 项目文档最低集合

```text
doc/implementation/<project>/
  README.md                         # 总目标、Phase DAG、状态
  <process-principles-or-link>.md   # 双线和验收规则
  phases/<phase>.md                 # Phase Contract + two-line obligations
  tasks/<phase>-execution-map.md    # 执行时滚动创建
  results/<phase>.md                # frozen candidate evidence / verdict
```

文档服务于执行，不反过来占据执行。一个 Phase 文档最重要的问题是：两条线共享什么合同、第一批代码是什么、
第一次可执行证明何时发生、哪些未知事实需要条件澄清、哪些共享选择才值得专门设计。
