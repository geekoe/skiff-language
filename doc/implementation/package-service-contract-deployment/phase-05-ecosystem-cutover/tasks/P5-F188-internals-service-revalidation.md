# P5-F188：Internals 服务生态复验

状态：Ready

## 直接父任务

- `P5-F180L-actor-full-chain-acceptance-result.md`

## 目标

使用当前 Skiff integration 对 Internals integration 的 Account、Codex Relay、AIHub、Agine 服务进行
真实 authoring/编译/测试，修复剩余 Package-owned schema 与 ServiceContract 消费问题。

## 必须实现

- 分别运行四个服务的既有测试和真实 compile/publish fixture；
- AIHub→Agine server-stream contract 使用 canonical in-process stream；
- Codex Relay HTTP/stream surface 不恢复旧 boundary schema；
- Account/Registry 类型引用来自 Package schema；
- 不操作 stable instance，不运行需要 stable 的 chat smoke；
- 仅修改 Internals 任务 worktree，独立提交和 result。

## 验证

- 四个服务各自测试通过；
- 真实 ServiceContract/Deployment/Assembly 生成通过；
- 旧 schema 正向符号零残留；
- diff check。

