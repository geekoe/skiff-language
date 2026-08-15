# Phase 运行手册（runbook）

每个 Phase 只按这 9 步执行。这是**唯一**的流程权威；其它文档是语义参考，不另立步骤。

1. **契约**：`phases/phase-N-*.md` 写三件事——本 Phase 支持面（含 fail-closed 面）、VCP、acceptance
   checklist。没有就写；写完语义不再改，只允许 Amendments。契约、architecture、decision 与 MAP 写到足以指导
   当前实现即可，不做全仓文档完备性 review，也不需要独立 PASS 才能开码。VCP = 一条 full-chain closure + **stage-sentinel
   矩阵**：同一组真实 fixture 在 source→admission、admission→emission、emission→atomic-link input、
   atomic-link→image、image→scheduler、scheduler→request→response 每个阶段边界各挂一个独立 test case；
   atomic-link input与image是同一constructor的输入/完成态sentinel，不是两个production API；哨兵输入必须是上一阶段
   真实生产边界的产出，不能 hand-build；首日全部 expected-red（真实断言，非 skip），一次 `--no-fail-fast`
   并行暴露所有已到达层的红。每个active Phase还必须导出单一transitive
   `phaseNWorkloadSpecs(root)`：直接组合上一Phase的workload specs，不嵌套旧Gate、不读取旧receipt；逐spec保留
   `testFormat`、`lanes`和`expectedTests`并用candidate-owned显式catalog记录source/parent/origin chain；不得解析嵌套
   id前缀作为provenance。唯一允许的命令归一化是给继承的`cargo test`
   幂等插入一次`--no-fail-fast`，不得改target/filter/harness args或给build/fmt/clippy插入。本Phase所有
   `testFormat != null` workload必须声明正整数`expectedTests`；继承spec若历史上缺该字段，必须保留并在handoff
   显式列出，不能猜默认值。
2. **执行地图**：`tasks/phase-N-execution-map.md` 一张表：lane / worktree / 写集 / join 顺序 / Gate 矩阵。
   这是写集的唯一权威，任何其它载体不重复文件清单。
3. **派发**：把本 Phase 写面拆成互不重叠的 lane，能拆几个就并行几个，**不设固定数量和固定角色模板**
   （Phase 4 的写面可能是 scheduler kernel / session-request owner / VM control，Phase 6 是 service/task/
   interface/callback/Actor 各 lane）。唯一硬约束：每个中央状态机只有一个 write owner；proof lane 独立。
   lane 数量由写面分区推导，不由角色名决定。派发前花几分钟做只读 gate-map 预调查：目标面会经过哪些
   pipeline 门、门在谁家，写进 MAP。任务信封 = 引用契约/MAP 条目 + 验收判据 + 预算 + 上报格式。写集外
   需求先上报，获准后**先改 MAP 再动代码**。
   任务信封的"验收判据"必须**引用契约的 VCP/checklist 小节，不得复述**；integrator 派单时机械核对
   信封判据 ⊆ 契约条款，防止复述漂移。
4. **验证**：focused 每轮跑；三包/全量只在 join 点跑；跨 worker cargo 用目录租约串行；每个`cargo test`
   workload带`--no-fail-fast`，build/fmt/clippy不带；>30s 重定向轮询，结果只跑一次。Cargo以外的独立
   fixture/checker/review可按MAP并行。
5. **收敛**：逐 gate 转绿；每扇门转绿时同一 join 收进 Gate 矩阵（含 fmt/clippy 自检命令）。**本 Phase 的
   新矩阵**（本 Phase 场景 + 上一 Phase 的 workload specs 作为回归子集）写完后、producer 未 join 时先完整跑一遍，
   留 expected-red baseline（非 zero/skip/ignore），证明矩阵可执行且覆盖契约全部 required scenario；这不是
   重跑上一 Phase 的 Gate，后续用它区分新旧红。outer runner捕获单个nonzero后继续全部剩余workload并为每项
   写receipt；zero-hit、skip/todo/ignored/cancelled、未执行项或runner提前退出都不是合格expected-red。
6. **Gate**：merged preflight 全绿 → freeze exact clean commit/tree → 新建 detached acceptance worktree。只有上一
   Phase result状态为`accepted`的exact commit/tree可作baseline；`candidate-pass`或旧receipt不能替代。
7. **Frozen candidate semantic review**（全新只读 agent）：只判实际代码/测试——核心契约落实、已接受 Phase
   不变式、假绿/第二权威/fallback、fail-closed。不审查 architecture 文档完备性，不因外围文档措辞漂移 FAIL；
   簿记和格式由 integrator 在 join 时机械检查。范围较大时按互斥主题并行读同一candidate，最后一次合并findings，
   不按“发现一个、修一个、再发现一个”串行扫描。所有reviewer返回前不开始修复；integrator只去重blocker并按
   owner一次批量派回。
8. **独立 Acceptance**（另一名全新只读 agent）：与第7步在freeze后并行启动，完整Gate + checklist + raw evidence
   核对；两个verdict都PASS后才写`results/phase-N.md`，其status必须是`accepted`，再合入main、push、清理worktree。
   任一FAIL都返回原write owner；修复后重新freeze，旧review/verdict/receipt不可复用。机械-only修复后的新验收信封
   可以只复核变更面 + 完整Gate，不整份重读。
9. **上报格式**（所有 lane）：`{完成了什么, 意外点, 尝试过什么, 需要什么}`。

强制隔离只有三条：frozen candidate semantic reviewer / Acceptance 必须是没写本 Phase 候选的全新 agent；proof 不修改生产制造 PASS；
kernel 状态机只有一个 write owner。

当前启动指针：Phase 5 accepted，baseline为
`094215c624712c257aa9455fc499cc6fb3657a9e` / `ec44479d88aca83f94038f84cf8a9c38f3693ba8`；Phase 6已由
[`phases/phase-6-cross-owner-execution.md`](./phases/phase-6-cross-owner-execution.md)与
[`tasks/phase-6-execution-map.md`](./tasks/phase-6-execution-map.md)激活并开始执行；Phase 7不得替Phase 6补
boundary/DB/recoverable/memory/GC语义。
