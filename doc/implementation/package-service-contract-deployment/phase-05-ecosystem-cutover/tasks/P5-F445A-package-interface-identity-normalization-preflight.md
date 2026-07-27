# P5-F445A Package interface identity normalization preflight

状态：Ready。只读、有界预检；不实现。

## 直接父节点

- `P5-F444C-agine-service-terminal-connect-only-cutover-result.md`

只从该 result 沿引用读取必要事实。不得把 Agine source cast、改call spelling或复制interface当作修复。

## 输入

| Repo | Root / commit |
| --- | --- |
| Skiff integration | `/Users/geek/workspace/skiff-phase-05-integration` / `c81266f3` |
| Internals integration | `/Users/geek/workspace/internals-phase-05-integration` / `19d4100` |
| F444C service draft | Internals stash commit `91f3cc32e9d6ce0b14b4145d3d94815ab1a52420` |
| skiff-packages integration | `/Users/geek/workspace/skiff-packages-phase-05-integration` / `19cfab5d` |

输入 worktree必须 clean。stash只读，不 apply/pop/drop。

## 要回答的问题

1. 建立最小、与 Agine 业务无关的 compiler fixture，精确复现：
   同 package id、symbol path、ABI hash的interface，因为
   `Dependency { dependency_ref }` 与 `PackageId { package_id }`表示不同而被判不等。
2. 从 source type resolution、PackageArtifact public callable、dependency binding、linked type plan、
   assembly/type comparison一路定位 expected 与 found identity的唯一生产 owner。
3. 判定正确修复位于：
   - package artifact/projection生成阶段；
   - consumer link/resolve阶段；
   - compiler semantic comparison阶段；
   - 或 Internals `packages/agent/**` 真的发布了错误identity。
   必须给证据，不能把同ABI hash当作无条件相等，也不能放宽不同package/version/build。
4. 搜索同类风险：public callable参数/返回值中的 `any interface`、nested nullable/array/record，以及
   dependency re-export。列出应一起覆盖而非逐个修Agine的矩阵。
5. 给出最小实现写集、RED/GREEN测试、可能的artifact identity/receipt影响和后继F444C恢复条件。

## 允许读取/运行

- Skiff `artifact-model/**`、compiler source/lowering/projection/driver/link相关代码与聚焦测试；
- Internals `packages/agent/**`、直接依赖的 package API/receipt，以及F444C stash中相关call site；
- 必要时运行不写artifact/stable的最小 compiler/package fixture或现有聚焦test listing。

不得运行完整 Internals canonical graph、stable/live/network，不得修改 production/test。

## 输出

只新增并提交：

`P5-F445A-package-interface-identity-normalization-preflight-result.md`

结论：

- `PREFLIGHT_COMPLETE / TASK_EXECUTABLE`
- `TASK_SCOPE_EXPANDED`
- `DESIGN_DECISION_REQUIRED`

必须包含精确owner、DAG/写集、独立复现fixture和验收矩阵。15分钟有界；无法形成单一路径时停止上报。
不得派子 Agent、merge/rebase/push。

worktree：

`/Users/geek/workspace/skiff-p5-f445a-interface-identity-preflight`

branch：

`codex/p5-f445a-interface-identity-preflight`
