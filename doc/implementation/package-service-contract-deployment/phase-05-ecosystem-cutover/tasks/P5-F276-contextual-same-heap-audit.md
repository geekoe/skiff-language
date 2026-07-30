# P5-F276 Contextual same-heap audit

状态：Ready（只读审计）。

## 直接父节点与权威链

- 直接父节点：
  `P5-F275-aihub-same-heap-diagnostic-result.md`
- analyzer 父结果：
  `P5-F271-container-projection-heap-cycle-precision-result.md`
- 唯一架构事实源：
  `doc/architecture/package-service-contract-deployment.md` 第 8 节

本任务只负责为一个通用、sound 的实现节点确定 owner 和数据模型，不修改代码。

## DAG 位置

- 输入代码状态：
  `codex/package-service-phase-05@a417b1de` 加本任务文档提交；production tree 等价于
  F271/F273 后的 `f69fb5c`。
- 当前成熟度：实现检查点。
- 后续：审计 result 成为 contextual same-heap 实现任务的直接父节点；该实现完成后解除
  AIHub 8/8 与 Agine/F269 总验收。
- F268I 正在另一个 worktree 集成 test-service/topLevel/loader；本任务不得修改或评价其文件。

## 审计范围

1. 逐跳解释 `requiresSameHeapIdentity`、`same_heap_identity_parameters`、return/direct
   provenance、heap store、native/builtin callable semantics 和 boundary eligibility 的现有职责。
2. 判断 identity requirement 应关联到：
   - 具体参数；
   - direct/reachable return projection；
   - 独立 identity observation；
   - 或其它结构化 owner。
3. 证明最小模型对父结果中的 AIHub 路径可以消除污染，同时对以下负例继续失败关闭：
   - `Map.get` 结果直接返回；
   - 结果写回 caller 图或逃逸；
   - identity/equality comparison；
   - conditional detached/alias；
   - unknown/dynamic target；
   - 跨 Package artifact replay；
   - SCC/递归传播。
4. 搜索所有设置/消费该 effect 的 production owner，确认 artifact/wire、build identity、Local
   ABI 和 operation contract 哪些会变化。
5. 给出单一有界实现任务：精确写入文件、非目标、聚焦测试和 fresh AIHub 8/8 风险探针。

## 工作边界

- 工作目录：
  `/Users/geek/workspace/skiff-p5-f276-same-heap-audit`
- 分支：
  `codex/p5-f276-same-heap-audit`
- 只读，不产生实现提交；审计结论返回主 Agent 落盘。
- 不修改 AIHub/Agine、std semantics、Package boundary eligibility 阈值或公共 DSL。
- 不添加函数名、Package ID 或调用链特判。
- 不操作 stable、不访问外网、不 push。

## 完成标准

返回可直接形成实现合同的审计结论，至少包含：

- 当前误报的精确 first-loss 点；
- 通用结构化模型及 soundness 论证；
- wire/identity 影响；
- 完整正负测试矩阵；
- 是否需要用户决策。

若 5 分钟内不能确认审计可执行，返回 `TASK_NOT_EXECUTABLE` 和最小缺口。

