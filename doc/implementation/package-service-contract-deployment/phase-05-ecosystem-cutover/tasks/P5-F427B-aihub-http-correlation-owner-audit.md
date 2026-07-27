# P5-F427B AIHub HTTP correlation owner audit

状态：Ready。只读审计。

## 直接父节点

- `P5-F427-http-correlation-field-removal-batch.md`

## 输入与读取范围

精确读取父节点记录的Internals integration，重点：

- `aihub/service/**`
- `aihub/client/**`
- AIHub external event/completions schemas、tests、receipt与README
- 为区分service-call/provider字段所需的`packages/llm-api/**`和AIHub依赖代码

禁止修改production/test/fixture。唯一写入是本leaf result。

## 必须回答

1. 枚举AIHub HTTP request body、SSE event envelope、error/terminal和browser state中的`request_id`、
   `requestId`、`runId`及同义字段，给出producer/consumer和实际用途。
2. 证明哪些字段只来自旧WebSocket correlation，应从HTTP request与stream item删除；不能仅因测试期待而
   保留。
3. 判断`runId`是否拥有独立业务语义或也只是`run-${requestId}`派生残留；若它被真正业务逻辑、取消、
   persistence或下游consumer使用，给出证据。若没有，明确是否应一起删除。
4. 区分external HTTP event wire与`managedLlm.streamChat` service-call/provider protocol；不能把后者的
   provider request/trace字段误删。
5. 给出删除后canonical event envelope、pre/post-start error、`[DONE]`与browser reducer/abort行为；不得
   发明新的correlation字段。
6. 列出ServiceProtocolIdentity、GatewayEntryIdentity、Package build、deployment/assembly和receipt
   哪些应变、哪些必须不变。
7. 给出单一或分离service/client repair leaf的精确写入范围、测试与反搜gate。

## 交付

新增并提交`P5-F427B-aihub-http-correlation-owner-audit-result.md`，包含字段/identity矩阵、repair DAG、
命令与clean状态。发现`runId`语义需要用户决定时返回精确`TASK_SCOPE_EXPANDED`，不得自行猜。不得修代码、
merge/rebase/push或访问stable/live/真实provider。

