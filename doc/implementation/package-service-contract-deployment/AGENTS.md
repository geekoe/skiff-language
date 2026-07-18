# Package / Service 分阶段实现约定

本目录落实唯一权威设计文档 `doc/architecture/package-service-contract-deployment.md`。设计文档定义四对象、
调用语义和长期不变量；这里的文档只把它拆成实现 DAG、任务文件、验证与验收，不得另行补充或修改架构语义。

## 执行规则

- 同一时间只细化和实现一个阶段。当前阶段验收并合并 `main` 后，才按最新 repo 事实细化下一阶段。
- 每阶段只向 `main` 合并一次；未经用户明确要求不 push。
- 主 Agent 负责拆分 DAG、关键路径、写入 ownership、风险、验收批次和 gate owner。任务数量不是目标；
  每阶段默认不超过三个实现波次，并优先填满三个 worker 槽位。
- 共享 schema、identity、wire 或公共 API 先作为短检查点落地；随后按 compiler、runtime、router、tooling
  等非重叠消费者扇出。禁止再次创建横跨多个顶层生产域的单一“大迁移任务”。
- 每个开发 Agent 使用独立 task worktree 和一个简短任务文件。任务文件必须引用权威设计条款，只能补充
  范围、依赖、命令、worktree 和证据 owner，不能成为第二份设计。
- 每阶段使用一个 integration worktree。task branch 从明确 checkpoint 创建，提交后合入 integration
  branch；阶段验收通过后才合入 `main` 并删除 worktree 与已合并分支。
- 发现设计缺口时暂停受影响 DAG 分支，询问用户或等待设计文档单独更新；不受影响分支继续执行。
- 每个阶段只能在自己负责的生产域落终态。尚未轮到的下游允许暂时不可用，
  但禁止为保持跨阶段可运行性新增 legacy/compatibility adapter、dual path、fallback 或临时
  authoring inference。未触碰的旧下游在其终态阶段直接替换；已切换的上游不再输出旧 DTO。
- Skiff 尚未发布，不兼容旧 artifact、manifest 或 CLI；禁止 dual-read、dual-write 和 runtime fallback。
- 直接触碰的重复规则、超长文件和职责混杂必须在当前任务处理；无关重构不进入本计划。

## 评审、验证与验收

- 阶段计划和任务文件完成后，默认由一个独立只读 Agent 做一次完整文档评审。只有高风险边界存在不同
  专业问题时才增加职责不同的 reviewer；不启动三个任务相同的 reviewer。
- blocking 文档修改影响 DAG 或公共检查点时才重审；命名偏好、完美化建议和非本阶段问题不触发循环。
- 开发 Agent 运行格式、静态检查和聚焦测试，并提交自验收矩阵：设计/任务条款、代码证据、反向搜索、
  测试命令。
- 中低风险 consumer 迁移按批次验收；高风险 schema/ABI/boundary 设独立验收。不是每个任务都单独创建
  验收 Agent。
- 昂贵完整 gate 对同一最终代码状态只设一个 owner、执行一次。验收 Agent 独立判断已有证据并按需做
  聚焦探针，不机械重跑完整 gate。
- 主 Agent 只做宏观接收：设计追溯、DAG、ownership、commit、证据有效性和集成条件。开发与验收已有
  owner 后耐心等待，不做影子实现、完整 code review 或重复测试。
- 阶段 gate 与独立阶段验收均 PASS 后，才合并 `main`。blocker 修复只使受影响证据和验收面失效。
