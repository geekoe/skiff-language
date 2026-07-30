# P5-F424A Skiff connect-only and outbound owner audit

状态：Ready。只读审计，风险高。

## 直接父节点

- `P5-F424-downlink-only-websocket-cutover-batch.md`

父节点已引用唯一权威设计并冻结HTTP上行、WebSocket下行语义。启动时默认只读本文和直接父节点；只有核对
具体约束时再沿引用向上读取。

## DAG位置与输入

这是F424第一波三个并行审计之一。精确Skiff输入为父节点记录的
`ba74febaca5dbe8f2b55d6db04e0544a6758bf4b`。结果将解除共享Skiff authoring/artifact/compiler与
router/runtime开发leaf；不解除consumer迁移或N5。

## 只读范围

允许读取整个Skiff repo，重点核对：

- service manifest authoring/parser与compiler driver；
- artifact-model、artifact-identity、deployment、RuntimeAssembly；
- compiler gateway projection与linked callable/adapter计划；
- `packages/std` WebSocket类型、native signature及compiler/runtime native owner；
- runtime protocol/frame、Host/eval `connection.send`；
- router WebSocket gateway、runtime endpoint/dispatcher、connection index与frame处理；
- 对应production tests、fixtures、generation oracle与tooling checks。

禁止修改production、test、fixture或设计。唯一允许写入是本任务result文档并提交。

## 必须回答

1. current compiler为何以及在哪里拒绝`service.websocket`；connect-only authoring最小current shape应由哪些
   已有DTO/parser owner承载，并如何机械保证每个service最多一个entry。
2. 从source callable到PackageArtifact typed ingress projection、ServiceDeployment、
   RuntimeAssembly、router manifest的完整connect-only记录链；哪些类型已存在、哪些已被F420B删除、哪些
   不能复活为service operation。
3. connect request/result的current std与native实现实际包含哪些legacy event/context/receive字段；按父设计
   删除后涉及哪些production/test owner。
4. `std.websocket.sendTextToBusinessIdentity`、binary/direct-connection等实际签名、挂起summary、
   runtime frame与router接收链；当前是否能从非WebSocket `ActivationContext`定位service和唯一entry，
   精确缺口在哪里。
5. router现有connection/policy/fan-out/generation pin/release cleanup中哪些仍可复用；客户端data frame
   当前流向何处，在哪个最窄owner实现close `1003`且证明零runtime dispatch。
6. 最小开发DAG：共享schema/compiler checkpoint与router/runtime consumer能否写入互斥；若不能，给出真实
   串行依赖和共享文件。
7. 每个建议leaf的精确写入范围、快速测试、真实test discovery命令、关键正负例、generation变化与最早
   cheap combined probe。
8. 全仓库legacy反向搜索词和命中分类，至少覆盖：
   `WebSocketIngressEvent`、`receiveEvent`、`websocketReceive`、`contextCodec`、
   `contextPayloadPresent`、Assembly WebSocket ingress、legacy operation与旧authoring `routes`。

## 停止条件与交付

若发现需要改变父设计、支持多个entry、暴露字符串entry id、恢复业务上行或增加新的公共语义，返回
`TASK_SCOPE_EXPANDED`，不得自行决定。否则新增：

`P5-F424A-skiff-connect-outbound-owner-audit-result.md`

Result必须包含精确commit/tree、owner表、调用链、缺口表、建议DAG/依赖、验证矩阵、反搜计数和worktree
状态。只提交result；不merge/rebase/push，不访问stable/live。

