# P5-F280 Open service error channel implementation audit

状态：Ready。

## 直接父节点与权威链

- 直接父结果：
  `P5-F279-open-service-error-channel-design-result.md`
- 父结果引用唯一架构、语言、publication与observability事实源。

启动时只读本任务；需要语义依据时沿父链向上读取。历史F274只保留旧实现事实，其declared-throws建议已失效。

## DAG位置与证据状态

- 权威checkpoint：Skiff integration commit `512135dd`。
- 当前节点：高风险跨层实现前只读审计；不是production实现或验收。
- 并行节点：F278 same-heap修正。不得修改或重新评审F278写入面。
- 完成后解除：共享artifact/language检查点、runtime/wire检查点、tooling与生态consumer任务的精确拆分。
- 本审计证据在相关source、artifact model、runtime error/wire、std/prelude或测试入口改变后失效；F278仅在
  same-heap限定范围内修改时不使本审计失效。

## 审计范围与必须回答的问题

只读追踪当前production链并提交一份result文档，至少回答：

1. 语言与std：
   - `ErrorPayload`从prelude registry、interface conformance、throw/catch leaf判定、std源码、syntax/VSC
     tooling进入哪些production入口；
   - 任意名义record、representation、named union及其synthetic/literal branch当前分别在哪里被接受或拒绝；
   - `std.error.InternalError`、其它platform error identity与目标`std.service.InternalError`的现状差异。
2. Artifact与compiler：
   - `PackageCallableSignature.throw_types`、
     `BoundaryOperationContract.errors`及相关normalization/validation/identity/dependency ingest的全部owner；
   - 哪些字段只服务旧public throw set，哪些仍被inline effect、控制流、throw provenance或runtime真实使用；
   - 删除字段后的canonical artifact、protocol identity与strict validation影响，不保留兼容读取。
3. Runtime与wire：
   - `UserException`创建、payload identity、catch matching、rethrow、service response error、in-process outbound
     service call、中间未处理传播与router response的完整跳点；
   - 现有linked type/schema materialization能否承载owner Package的public `SchemaClosed` error，缺少哪些
     record lookup或encoded-payload owner；
   - stack/source当前在哪里创建、为什么为空或不完整，diagnostic frame如何进入wire，以及
     `traceId/errorId`和telemetry的现有owner；
   - 固定`InternalError`生成一次、同值跨中间service传播、每跳新建本地exception stack的最小共享抽象。
4. 测试与生态：
   - test runner inline effects/doubles、compiler/runtime fixture、std source与public生态中依赖
     `ErrorPayload`、`throw_types`或`UnhandledServiceError`的同类残留；
   - 区分必须迁移的production/fixture与仅保留历史文字的implementation记录；
   - 给出package-local private error、同包public error、第三方Package public error、非closed/编码失败、
     中间service透明传播、精确catch、每跳栈和脱敏的最小真实路径矩阵。

## 交付格式

新增：

`P5-F280-open-service-error-channel-implementation-audit-result.md`

结果必须包含：

- 按真实执行顺序列出的production跳点、当前形状与首次语义损失；
- 可删除、可复用和必须新增的canonical owner，明确禁止复制的owner；
- 最多三个实现波次的DAG，每个节点的直接依赖、非重叠production/test写入范围、风险与验收分组；
- 最早可运行的便宜风险探针、最终跨层正负探针和昂贵gate唯一owner建议；
- 仍需用户决定的公共设计缺口；若没有，明确写“没有新增设计决策”；
- 对每个结论给出文件与symbol证据，不以搜索命中数量代替owner判断。

## 非目标与禁止范围

- 不修改任何production、fixture、std源码、reference/architecture或其它任务文件。
- 不实现`throws`语法，不从函数体推导公开throw set。
- 不增加兼容field、dual read/write、legacy adapter或结构猜测。
- 不运行完整测试、build、stable instance、生态发布或live命令。
- 不修改skiff-packages、internals或任何F278文件。

允许只修改并提交本任务的result文档。

## Worktree、分支与提交

- worktree：`/Users/geek/workspace/skiff-p5-f280-error-audit`
- branch：`codex/p5-f280-error-audit`
- base：包含本任务文件的integration checkpoint。
- 完成后提交result文档，不push，不合入其它分支，不清理别人的worktree。
- 这是一次性有界审计会话；交付result后不得自行承接实现节点。
