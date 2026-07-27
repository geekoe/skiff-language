# P5-F426 Connect wire and HTTP consumer wave

状态：Ready。F425第一波已合流；当前仍是实现检查点。

## 直接父节点

- `P5-F425-downlink-websocket-implementation-checkpoint.md`
- `P5-F425A-skiff-websocket-authoring-compiler-checkpoint-result.md`
- `P5-F425B-aihub-http-stream-service-result.md`
- `P5-F425C-aihub-http-stream-client-result.md`
- `P5-F425D-agine-user-http-service-checkpoint-result.md`

## 精确输入

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `6bd1697bc7be1b7119a3157146a2d313f4bffc4e` | `723467b529da07c145201bfa1637b309081c522d` |
| Internals integration | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` |
| skiff-packages integration | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

所有integration worktree在本wave建立时clean。

## 已建立与未建立的事实

- F425A已生成strict singleton WebSocket authoring、connect-only typed surface、optional handler、
  canonical internal entry ID和ServiceDeployment/RuntimeAssembly entry。
- runtime Host admission仍刻意拒绝WebSocket，Router仍没有current Assembly connect gateway；这是后继
  consumer，不是F425A失败。
- F425A production checks通过；test-runner两个optional-handler fixture属于后继D4。
- AIHub service/client已迁到HTTP event stream并删除全部AIHub WebSocket。两边局部测试通过，尚未在同一
  current Skiff/Internals状态完成canonical combined。
- Agine service已增加22个ordinary-user HTTP operation；browser仍在用legacy WebSocket RPC。
- Agine Host/host-file分支继续等待用户确认，不得由本wave猜测。

## 并行DAG

```text
F426A  Skiff current websocketConnect request/response wire
F426B  AIHub merged-state read-only combined probe
F426C  Agine ordinary browser caller HTTP migration
```

F426A完成后解除Runtime与Router并行consumer。F426B若暴露当前blocking source错误，只记录一次精确failure
classification并由新repair节点处理。F426C只解除ordinary browser分支；Host与legacy receive cleanup仍
阻塞。

本wave不运行stable/live/instance、完整N5或最终gate，不merge/rebase/push。

