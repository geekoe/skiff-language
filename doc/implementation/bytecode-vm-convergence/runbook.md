# Phase 运行手册（runbook）

每个 Phase 只按这 9 步执行。这是**唯一**的流程权威；其它文档是语义参考，不另立步骤。

1. **契约**：`phases/phase-N-*.md` 写三件事——本 Phase 支持面（含 fail-closed 面）、VCP、acceptance
   checklist。没有就写；写完语义不再改，只允许 Amendments。契约、architecture、decision 与 MAP 写到足以指导
   当前实现即可，不做全仓文档完备性 review，也不需要独立 PASS 才能开码。VCP = 一条 full-chain closure + **stage-sentinel
   矩阵**：同一组真实 fixture 在 source→admission、admission→emission、emission→atomic-link input、
   atomic-link→image、image→scheduler、scheduler→request→response 每个阶段边界各挂一个独立 test case；
   atomic-link input与image是同一constructor的输入/完成态sentinel，不是两个production API；哨兵输入必须是上一阶段
   真实生产边界的产出，不能 hand-build；首日全部 expected-red（真实断言，非 skip），一次 `--no-fail-fast`
   并行暴露所有已到达层的红。closure-only 且没有 production producer 的 Phase 按第 5 步的受控红例外执行。
2. **执行地图**：`tasks/phase-N-execution-map.md` 一张表：lane / worktree / 写集 / join 顺序 / Gate 矩阵。
   这是写集的唯一权威，任何其它载体不重复文件清单。写集是 provisional decomposition boundaries，不是 immutable
   file locks；真实 seam 所需少量跨 owner 写入完成后，在 task handoff 中报告为实际 write set，由 integrator
   验证并反映到下一次 MAP amendment。
3. **派发**：把本 Phase 写面拆成互不重叠的 lane，能拆几个就并行几个，**不设固定数量和固定角色模板**
   （Phase 4 的写面可能是 scheduler kernel / session-request owner / VM control，Phase 6 是 service/task/
   interface/callback/Actor 各 lane）。硬约束包括：同一 worktree 不同时并发写入；proof lane 不修改生产制造
   PASS；每个中央状态机在任一时刻只有一个 write owner，但该 owner 可在真实收敛后通过 MAP amendment 调整。
   lane 数量由写面分区推导，不由角色名决定。派发前花几分钟做只读 gate-map 预调查：目标面会经过哪些
   pipeline 门、门在谁家，写进 MAP。任务信封 = 引用契约/MAP 条目 + 验收判据 + 预算 + 上报格式。写集是初始分解
   边界，不是文件锁；真实 seam 需要少量跨 owner 写入时，可完成写入并在 task handoff 中作为实际 write set 上报，
   integrator 验证后在下一次 MAP amendment 中反映。新语义、第二 authority 或兼容路径仍必须先上报再改。
   任务信封的"验收判据"必须**引用契约的 VCP/checklist 小节，不得复述**；integrator 派单时机械核对
   信封判据 ⊆ 契约条款，防止复述漂移。
4. **验证**：focused 每轮跑；三包/全量只在 join 点跑；跨 worker cargo 用目录租约串行；>30s 重定向轮询；
   结果只跑一次。
5. **收敛**：逐 gate 转绿；每扇门转绿时同一 join 收进 Gate 矩阵（含 fmt/clippy 自检命令）。**本 Phase 的
   新矩阵**（本 Phase 场景 + 上一 Phase 导出的 canonical workload specs 作为回归子集；不 child-run 旧 Gate、
   不复用旧 PASS receipt）写完后、producer 未 join 时先完整跑一遍，
   留 expected-red baseline（非 zero/skip/ignore），证明矩阵可执行且覆盖契约全部 required scenario；这不是
   重跑上一 Phase 的 Gate，后续用它区分新旧红。
   唯一例外是 Phase Contract 明确声明 **closure-only 且没有 production producer join**：不得为了满足流程故意
   破坏 production 制造红；Proof Line 必须用受控 command failure、missing receipt 或 tamper self-test 证明 Gate
   nonzero FAIL、证据检查 fail closed，且早期红不截断所有后续可达命令。真实 whole-system baseline 可以直接 green；
   一旦新增 production observability 或 enforcement producer，与该 producer关联的真实场景仍必须在 join 前留下
   nonzero/non-skip expected-red。
6. **Gate / freeze**：merged preflight 全绿 → freeze（exact commit/tree/status + evidence epoch）。失败或中断的
   evidence directory只读保留；不得原地补 receipt/resume，重跑必须使用新的 absent directory。此时不提前创建
   Acceptance worktree。
7. **同 HEAD 并行 review 与批量修复**：同时启动多名全新只读 agent，至少分 semantic implementation 与
   proof/Gate/evidence 两面，全部读取同一 frozen commit/tree；发现 blocker 仍完成本 scope，不得报一个就修一个。
   Integrator 等全部返回后合并、去重并封存唯一 blocker ledger。若非空，先 unfreeze、按原 semantic/proof owner
   和 exact write set 一次并行修完整批次，再跑 affected focused checks + full merged preflight、re-freeze 新 epoch；
   严格落在 sealed fix scope 内的变更可并行 targeted recheck；已上报并经 integrator 验证的跨 owner 写入在下一
   MAP amendment 记录为实际 write set，未上报的 write-set 逃逸必须重跑完整 fresh cohort。Review 只判实际代码/测试、
   核心契约、已接受不变式、假绿/第二权威/fallback/fail-closed；不做
   architecture 文档完备性 review，不因外围措辞漂移 FAIL。
8. **独立 Acceptance 与 closeout**：blocker ledger 在 exact frozen HEAD 上为零后，另一名全新只读 agent 在新建
   detached clean worktree运行完整 Gate + checklist + raw evidence核对；它不能是 writer或本轮 reviewer。
   FAIL 回到第 7 步并产生新 freeze/evidence epoch；PASS 后才写 `results/phase-N.md` 与 status-only closeout，确认
   main checkout clean/on `main` 且可安全合流，合入 main → push → 把 raw evidence 留在可删除 worktree之外并记录
   hash → 按 exact inventory归档/删除本 Phase worktree、stash、active branch/ref。dirty/unmerged 状态不得强删；
   先提交、取得明确丢弃授权，或固定到已验证的 archive ref。禁止 wildcard 清理和触碰其它 Phase。Terminal Phase
   最后置为 `closed/accepted`、停止所有 agent且不自动启动下一 Phase。
9. **上报格式**（所有 lane）：`{完成了什么, 意外点, 尝试过什么, 需要什么}`。

强制硬约束是：frozen candidate reviewer cohort / Acceptance 必须是没写本 Phase 候选的全新 agent，且
Acceptance 与 cohort 不复用 owner；proof 不修改生产制造 PASS；同一时刻 kernel 状态机只有一个 write owner，
但该 authority 可在真实收敛后通过 MAP amendment 调整。
