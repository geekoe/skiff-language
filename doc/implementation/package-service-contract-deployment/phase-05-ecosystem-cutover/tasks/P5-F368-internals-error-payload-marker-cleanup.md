# P5-F368 Internals package ErrorPayload marker cleanup

状态：Ready（Relay真实发布前置；纯package source，与F365/F366/F367写入不重叠）。

## 直接父节点

- `P5-F287-std-error-surface-migration-result.md`
- `P5-F291-open-error-compiler-consumer-checkpoint-result.md`
- 当前生态迁移DAG：`P5-H36-external-ingress-implementation-dag.md`

父节点已冻结：语言不存在`ErrorPayload` marker；任意符合语言规则的自定义名义值可在package内部抛出。本任务
只迁移仍使用旧marker的Internals package source，不重新设计错误类型或跨service error channel。

## Exact base与已确认范围

- Internals integration：`14ccfd417c9f45f00bd77015494cdd727e0f88dc`
- tree：`2327a766bcc6f32e7470b57420659fc991ef8a15`
- Skiff toolchain：使用包含本task的
  `/Users/geek/workspace/skiff-phase-05-integration`，返回实际commit/tree。
- 当前package source精确命中：
  - `packages/llm-api/decode.skiff`：1；
  - `packages/agent/{drain,thread_runtime_support,tools,runner}.skiff`：7。

Agine service中的两个命中归后续Agine service migration owner；skiff-packages Registry的两个命中由F369
负责，均不得在本leaf修改。

## 必须完成

1. 对上述8个声明只删除`implements ErrorPayload`，保留type名、字段、discriminator/representation、
   throw/catch使用和业务行为。
2. Internals `packages/llm-api`与`packages/agent` production source中
   `implements ErrorPayload`必须为零；不得创建replacement marker、alias或空interface。
3. 使用fresh isolated artifact root验证真实package链：
   - bootstrap canonical std；
   - 从`/Users/geek/workspace/skiff-packages-phase-05-integration`依序发布`http-session`与`track`；
   - 依序发布Internals `llm-api`、`llm-providers`、`agent`；
   - 保存每个真实PackageArtifact receipt及最终非零build identity。
4. 运行受影响package的现有聚焦Node/source tests；若不存在独立命令，至少枚举并运行对应
   `node skiff ... test`的非live package tests。不得把零测试或只做文本搜索当作全部证据。

## 写入与停止边界

允许写入仅为上述四个Agent文件和`packages/llm-api/decode.skiff`。禁止修改Agine、Relay、Account、
skiff-packages、Skiff compiler/std/runtime、共享workflow、stable/live或外部服务。

若删除marker后暴露的下一失败需要改字段、throw/catch语义、共享compiler或其它owner，返回
`TASK_SCOPE_EXPANDED`并保留可独立成立的scoped commit。

- worktree：`/Users/geek/workspace/internals-p5-f368-error-payload-marker-cleanup`
- branch：`codex/p5-f368-error-payload-marker-cleanup`
- production/tests一个commit；clean，不merge/rebase/push。
- 启动5分钟内开始修改；返回exact commit/tree、changed files、非零测试与真实receipt摘要。
