# P5-F427A Agine HTTP correlation owner audit

状态：Ready。只读审计。

## 直接父节点

- `P5-F427-http-correlation-field-removal-batch.md`

## 输入与读取范围

精确读取父节点记录的Internals integration，重点：

- `agine/protocol/**`
- `agine/service/**`全部HTTP payload/response/route/adapter及tests
- `agine/client/src/lib/http.ts`和ordinary caller/tests
- F426C WIP checkpoint（若已提交）只作为差分参考，不作为候选

允许为归属判断读取legacy WebSocket代码、Host与E2E helper；禁止修改任何production/test/fixture。唯一
写入是本leaf result。

## 必须回答

1. 枚举Agine全部HTTP request/response schema中的`requestId`、`request_id`、correlation同义字段，不只检查
   F425D新增22项，也检查原14项。
2. 对每个命中给出producer、consumer、是否实际读取、是否只用于旧WS response匹配，以及删除后的精确wire
   shape。
3. 区分legacy WS requestId和HTTP字段；说明旧receive仍存在期间怎样避免共用DTO把requestId重新带回HTTP。
4. 核对HTTP response wrapper是否仍返回`eventName`、`requestId`或`*-response`模拟字段；按用户决定给出
   应删除的闭集。HTTP status/body仍需表达业务成功/错误，但不能依赖旧correlation envelope。
5. 核对日志、测试mock与E2E helper是否把requestId当transport key；指出需要改为HTTP调用栈局部状态的owner。
6. 证明`runId`、toolCallId、attemptId、chatId、agentId等真实业务ID不应误删；若有幂等需求，给出已有
   canonical字段而不是自行新增。
7. 给出最小repair DAG：service/protocol checkpoint与browser caller是否必须串行，F426C WIP哪些hunk可
   安全复用，精确写入范围、测试与反搜gate。

## 交付

新增并提交`P5-F427A-agine-http-correlation-owner-audit-result.md`，包含完整字段矩阵、repair DAG、命令与
clean状态。发现需要新业务ID或改变幂等语义时返回`TASK_SCOPE_EXPANDED`。不得修代码、merge/rebase/push或
访问stable/live。

