# P5-F424B Agine HTTP uplink owner audit

状态：Ready。只读审计，风险高。

## 直接父节点

- `P5-F424-downlink-only-websocket-cutover-batch.md`

父节点已引用唯一权威设计并冻结HTTP上行、WebSocket下行语义。

## DAG位置与输入

这是F424第一波三个并行审计之一。精确Internals输入为
`eddeeb8615057233a8a9ba2fbcf748d863d23e3b`，Skiff工具链输入为
`ba74febaca5dbe8f2b55d6db04e0544a6758bf4b`。结果将解除Agine service/client/host迁移leaf；
不解除共享Skiff实现或N5。

## 只读范围

允许读取：

- `internals-phase-05-integration/agine/service/**`
- `internals-phase-05-integration/agine/client/**`
- `internals-phase-05-integration/agine/host/**`
- Agine相关scripts、tests、receipts和package manifests
- 为核对HTTP/WebSocket std形状所需的Skiff只读文件

禁止修改production、test、fixture或设计。唯一允许写入是本任务result文档并提交。

## 必须回答

1. 逐项枚举server `agine_ws_dispatch`及其下游实际接受的所有`eventName`，标记browser、Host、test-only
   producer与业务效果；不要只列入口文件。
2. 逐项枚举browser/Host production WebSocket发送点，映射到server event或证明dead/unreachable。
3. 对每项上行操作标记：
   - 已有HTTP route和handler可原样复用；
   - 有内部业务函数但缺HTTP route/adapter；
   - 只有WebSocket协议层实现，需要提取共享handler；
   - 可以删除的ping/legacy/test-only行为。
4. 特别核对Host的activation、hello、heartbeat、tool result、host file list/search result、
   current-directory与tool-attempts；给出迁到HTTP后认证/host identity从哪里取得，不能用WebSocket
   connection context补事实。
5. 枚举所有服务端下行路径、eventName和真实消费者，确认它们在移除receive后仍需要WebSocket；核对connect
   handler如何从upgrade request建立browser/host `businessIdentity`与connection policy。
6. 判断现有14个HTTP entry是否覆盖迁移；若不足，按当前Agine HTTP dispatcher约定给出最小新增path/method
   分组和对应source owner。这里只记录repo事实与最小机械建议，不修改公共设计。
7. 给出可互斥开发拆分：service HTTP handlers、browser callers、Host callers/tests是否可并行，哪些共享
   protocol/types文件形成串行checkpoint。
8. 给出每个leaf的精确写入范围、聚焦测试、真实test discovery命令、关键正负例和legacy反向搜索词。

## 停止条件与交付

若HTTP迁移要求新的平台认证机制、改变Host安全边界、丢失业务行为或出现多个等价但会改变产品语义的协议方案，
返回`TASK_SCOPE_EXPANDED`并列出精确决策，不得自行选择。否则新增：

`P5-F424B-agine-http-uplink-owner-audit-result.md`

Result必须包含精确commit/tree、完整event/caller矩阵、HTTP覆盖/缺口、downlink/connect事实、建议DAG、
验证矩阵和worktree状态。只提交result；不merge/rebase/push，不访问stable/live。

