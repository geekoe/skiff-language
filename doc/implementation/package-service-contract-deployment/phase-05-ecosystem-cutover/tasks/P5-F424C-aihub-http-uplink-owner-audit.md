# P5-F424C AIHub HTTP uplink owner audit

状态：Ready。只读审计，风险高。

## 直接父节点

- `P5-F424-downlink-only-websocket-cutover-batch.md`

父节点已引用唯一权威设计并冻结HTTP上行、WebSocket下行语义。

## DAG位置与输入

这是F424第一波三个并行审计之一。精确Internals输入为
`eddeeb8615057233a8a9ba2fbcf748d863d23e3b`，Skiff工具链输入为
`ba74febaca5dbe8f2b55d6db04e0544a6758bf4b`。结果将解除AIHub service/client迁移leaf；
不解除共享Skiff实现或N5。

## 只读范围

允许读取：

- `internals-phase-05-integration/aihub/service/**`
- `internals-phase-05-integration/aihub/client/**`
- AIHub相关scripts、tests、receipts和package manifests
- 为核对HTTP raw stream形状所需的Skiff只读文件

禁止修改production、test、fixture或设计。唯一允许写入是本任务result文档并提交。

## 必须回答

1. 枚举AIHub WebSocket connect/receive的production行为和所有客户端发送点；精确说明`chat.request`
   request、stream item、terminal/error与取消/断开语义。
2. 将WebSocket chat请求映射到现有`/v1/chat/events`、`/chat/events`、
   `/v1/chat/completions`、`/chat/completions`实现，确认哪些HTTP entry已提供等价stream wire，哪些字段或
   terminal语义不同。
3. 枚举AIHub WebSocket所有下行行为与真实客户端消费者；判断迁移上行后是否仍有必须保留的主动push。
   若没有，给出删除AIHub WebSocket entry的repo证据；若有，说明connect/business identity/policy需求。
4. 核对browser HTTP client是否已有stream parser、abort/cancel和错误处理能力，最小迁移写入点在哪里。
5. 核对service receipt、source tests、client tests/E2E与manifest oracles需要怎样更新；不得把
   service-call `managedLlm.streamChat`和external HTTP stream混为同一surface。
6. 给出service/client是否可并行实现、共享protocol/types checkpoint、精确写入范围、聚焦测试、真实test
   discovery命令、关键正负例和legacy反向搜索词。

## 停止条件与交付

若现有HTTP stream无法保持客户端可观察语义、需要新外部协议或AIHub仍有独立主动push需求且身份模型不明确，
返回`TASK_SCOPE_EXPANDED`并列出精确差异，不得自行选择。否则新增：

`P5-F424C-aihub-http-uplink-owner-audit-result.md`

Result必须包含精确commit/tree、WebSocket/HTTP行为对照、客户端迁移面、是否保留AIHub下行entry的结论、
建议DAG、验证矩阵和worktree状态。只提交result；不merge/rebase/push，不访问stable/live。

