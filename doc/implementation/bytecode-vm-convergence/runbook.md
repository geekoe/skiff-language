# Phase 运行手册（runbook）

每个 Phase 只按这 9 步执行。这是**唯一**的流程权威；其它文档是语义参考，不另立步骤。

1. **契约**：`phases/phase-N-*.md` 写三件事——本 Phase 支持面（含 fail-closed 面）、VCP、acceptance
   checklist。没有就写；写完语义不再改，只允许 Amendments。VCP = 一条 full-chain closure + **stage-sentinel
   矩阵**：同一组真实 fixture 在 source→admission、admission→emission、emission→link、link→verify、
   verify→scheduler、scheduler→request→response 每个阶段边界各挂一个独立 test case；哨兵输入必须是上一阶段
   真实生产边界的产出，不能 hand-build；首日全部 expected-red（真实断言，非 skip），一次 `--no-fail-fast`
   并行暴露所有已到达层的红。
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
4. **验证**：focused 每轮跑；三包/全量只在 join 点跑；跨 worker cargo 用目录租约串行；>30s 重定向轮询；
   结果只跑一次。
5. **收敛**：逐 gate 转绿；每扇门转绿时同一 join 收进 Gate 矩阵（含 fmt/clippy 自检命令）。**本 Phase 的
   新矩阵**（本 Phase 场景 + 上一 Phase 的 Gate 作为回归子集）写完后、producer 未 join 时先完整跑一遍，
   留 expected-red baseline（非 zero/skip/ignore），证明矩阵可执行且覆盖契约全部 required scenario；这不是
   重跑上一 Phase 的 Gate，后续用它区分新旧红。
6. **Gate**：merged preflight 全绿 → freeze（exact commit/tree）→ 新建 detached acceptance worktree。
7. **独立 review**（全新只读 agent）：只判语义——契约落实、已接受 Phase 不变式、假绿/第二权威/fallback、
   fail-closed。不判簿记和格式（那是 integrator 在 join 时的机械检查）。
8. **独立 Acceptance**（全新只读 agent）：完整 Gate + checklist + raw evidence 核对。PASS → 写
   `results/phase-N.md` → 合入 main → push → 清理 worktree。机械-only 的 FAIL 修复后，新 agent 的验收信封
   可以只复核变更面 + 完整 Gate，不整份重读。
9. **上报格式**（所有 lane）：`{完成了什么, 意外点, 尝试过什么, 需要什么}`。

强制隔离只有三条：reviewer / Acceptance 必须是没写本 Phase 候选的全新 agent；proof 不修改生产制造 PASS；
kernel 状态机只有一个 write owner。
