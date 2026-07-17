# Package / Service 分阶段实现约定

本目录落实 `doc/architecture/package-service-contract-deployment.md`。架构文档定义终态；这里的
实现文档只定义阶段、DAG、任务边界与验收证据，不得反向修改四对象模型。

## 执行规则

- 同一时间只细化一个阶段。当前阶段验收并合并 `main` 后，才根据实际代码状态细化下一阶段。
- 每个开发任务有一个独立任务文件；开发 Agent 和对应验收 Agent 必须使用同一文件作为权威输入。
- 每个阶段使用一个 integration worktree。可并行任务从同一已提交 checkpoint 建独立 task worktree，
  各自提交后再合并到 integration branch。
- task branch 可以有多个提交；integration branch 只在阶段完成后向 `main` 合并一次。未经用户要求不
  push。
- Agent 不得修改未授权的任务文件或借集成修复引入新语义。发现架构阻塞时，把它升级成 DAG 前置
  任务并更新文档；无法从 canonical 架构唯一推导时才询问用户。
- 临时 adapter 只能转换数据形状，不能复制 identity、type/effect、validation 或 artifact projection
  规则。任务文件必须写明删除阶段，结构 gate 必须阻止 adapter 扩散。
- Skiff 尚未发布，不兼容旧 artifact、manifest 或 CLI。阶段内可以重写 fixture；禁止 dual-read、
  dual-write 或 runtime fallback。
- 测试按任务风险聚焦运行，昂贵 gate 每个最终代码状态只指定一个 owner。测试通过不能代替结构证据。
- 直接触碰的重复规则、超长文件和职责混杂必须在当前阶段清理；无关重构不进入本计划。

## 评审与验收

- 阶段实现前，由三个独立只读 Agent 完整评审当前总体计划、当前阶段计划和全部当前阶段任务文件。
- 默认只做一轮；只有 blocking issue 被采纳并修改文档后才重新完整评审。
- 开发 Agent 必须提交自验收矩阵：Contract 条款、代码证据、反向搜索证据、测试命令。
- 主 Agent 复核集成候选后，再启动未参与开发的独立只读验收 Agent。
- 阶段 gate 与独立验收均 PASS 后，才合并 `main`、删除 task/integration worktree 和已合并分支。
