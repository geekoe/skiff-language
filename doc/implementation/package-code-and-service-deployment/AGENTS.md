# Package Code / Service Deployment 实施约定

本目录继承仓库根 `AGENTS.md`。这里的文档是阶段性实施计划，不是长期架构契约；
长期结论必须同步到 `doc/architecture/` 或 `doc/reference/`。

## 文档状态

- `implementation-plan.md` 是总体阶段、执行纪律和阶段验收边界的唯一入口。
- 只有标为 `ready` 的阶段目录可以派发实现 Agent。
- `outline-only` 阶段只有方向、进入条件和验收目标，不能据此直接开发。
- 前一阶段验收并合并 `main` 后，才允许细化下一阶段；届时可以拆分、合并或调整后续阶段。
- 不创建 `README.md` 作为第二入口。

## Agent 与任务

- 一个实现/集成任务文件对应一个 Agent、一个 worktree、一个提交；`Axx` 只读验收任务只输出
  报告，不创建提交。
- Agent 开工前必须完整阅读总体计划、当前阶段计划和自己的任务文件。
- Agent 只完成任务文件授权的范围；依赖未完成、契约不清或需要扩大范围时停止并上报，不能自行跨 DAG 节点实现。
- 开发 Agent 不负责给自己的任务做独立验收。阶段集成后由单独的验收 Agent 只读验收。
- 每个开发/集成 Agent 必须提交代码或任务证据；不 push，除非用户明确要求。

## Worktree 与集成

所有 worktree 直接建立在 `/Users/geek/workspace` 下，不嵌套到仓库或其它 worktree 中。

每阶段在任何任务开工前建立一个集成 worktree/branch，例如：

```text
/Users/geek/workspace/skiff-phase-01
codex/package-service-phase-01
```

阶段集成协调 Agent 从阶段开始持续拥有该 worktree：每一批任务完成后合并其提交、运行最小
check并公布唯一checkpoint commit。任务 worktree只从已经包含其全部前置节点的checkpoint创建，
不得由任务Agent自行cherry-pick多个前置分支，例如：

```text
/Users/geek/workspace/skiff-p1-t06
codex/package-service-p1-t06
```

同层无依赖任务可以并行。冲突在集成worktree由协调Agent解决，不能让两个任务Agent同时修改
同一个worktree；语义冲突退回原任务，不由协调Agent发明。最终阶段gate是该持续协调任务的
完成条件，不表示到最后才第一次集成。阶段验收通过后：

1. 把阶段分支合并回 `main`；
2. 删除阶段与任务 worktree；
3. 删除已合并临时分支；
4. 再开始细化下一阶段。

## 营地原则硬门禁

实现前先用 `rg` 找现有规则 owner。出现以下任一情况时，功能实现必须暂停，先升级为
独立前置任务：

- 同一解析、校验、identity、projection、link 或 dispatch 规则需要在两个位置同步实现；
- 为新模型复制旧 package/service 分支，再靠测试保持一致；
- 新代码必须继续向职责混杂的超长文件追加核心逻辑；
- 阶段间只能通过字符串、raw JSON 或隐式字段约定传递本应是 typed fact 的信息。

生产文件超过约 800 行、核心函数超过约 150 行，且本任务要实质修改其职责时，先按
领域职责拆分再加功能。纯机械改名或删除旧字段可以说明理由后不单独拆分；测试/golden
文件不按数字机械切片，但出现多个独立行为域时也要按域拆开。不要借机重构不在本次数据
流上的代码。

## 测试纪律

- 任务 Agent 只运行任务文件列出的聚焦测试，通常 1–3 组命令；不得默认运行全仓测试。
- 阶段集成 Agent 运行当前阶段的组合 gate。
- `pnpm test` / `pnpm verify` 只在总体计划指定的阶段或最终验收运行。
- 删除旧架构测试是允许的，但必须说明它验证的是哪条已删除语义，并用新模型的行为测试
  覆盖仍然成立的不变量。
- 不为了保持测试数量而保留重复 fixture、兼容 adapter 或旧生产路径。

## 架构问题与评审标准

- `recoverable value`、即时 service boundary、callback capability、state/config owner 等概念
  如果无法从 canonical 文档得到唯一结论，必须形成独立前置任务；前置任务仍无法裁决时
  直接询问用户。
- 文档和阶段评审以“方向成立、DAG 可执行、阶段可验收”为通过线，不追求覆盖所有未来
  细节。
- 默认只做一轮完整评审；只有 blocking issue 才修改并再做一轮。第二轮仍有无法裁决的
  blocking issue 时停止并询问用户，不无限循环。
- reviewer 发现的架构问题不能塞进某个开发任务顺手修，必须升级为 DAG 前置节点。
- 命名、错误文案、非关键 fixture 组织等不影响正确性的意见记为 non-blocking，不阻止开工。
