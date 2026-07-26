# P5-F389 Compiler→Router HTTP fixture migration

状态：Ready（S5 HTTP-only）。

## 直接父节点

- `P5-F350-external-ingress-ecosystem-migration-audit-result.md`
- `P5-F357-http-gateway-compiler-projection-result.md`

父节点已冻结
`compiler/tests/fixtures/router-websocket-fixture`中的`/ping`是external-only HTTP，不是service-call
operation。本节点只迁移这一条HTTP fixture；目录名或相邻WebSocket fixture不授权修改WS协议。

## Worktree

- `/Users/geek/workspace/skiff-p5-f389-compiler-router-http-fixture`
- branch `codex/p5-f389-compiler-router-http-fixture`
- base：包含本任务的Skiff phase-05 integration。

## 必须完成

1. 保持现有`ping() -> string`实现和method/path/host逐值不变。
2. 从Package API移除external-only `ping` service-call/public projection，使ServiceContract
   `1 -> 0`；handler仍是当前implementation package中的exact private callable。
3. 把旧`routes/operation`authoring改为一个named HTTP gateway entry：
   - `kind: typedJson`
   - unary
   - 零request arguments/external sources
   - response schema String
   - stable readable key
4. fresh compile/publish/build-only证明：
   - service-call roots/operations/bindings为0；
   - gateway entries/ingress为1；
   - selector、key、handler、identity、mode/reference闭合。
5. 更新compiler→Router generated artifact compatibility断言，使其读取gateway entry/identity而不是
   contract operation；保留真正WebSocket断言，不顺手迁移或删除。

## 写入边界

允许：

- `compiler/tests/fixtures/router-websocket-fixture/**`
- 该fixture的直接compiler/Router compatibility tests与局部receipt fixture。

禁止：

- compiler projection production、artifact DTO/identity；
- Router production、Host/runtime/test-runner；
- 其它service/fixture、WebSocket协议；
- stable/live。

若HTTP fixture无法在不改变WS shared DTO的情况下独立通过，返回`TASK_SCOPE_EXPANDED`并给出精确断点。

运行所有direct compiler fixture/Router compatibility测试、fresh真实artifact验证、反搜旧HTTP
`operation`字段及`git diff --check`。写
`P5-F389-compiler-router-http-fixture-migration-result.md`，production/tests/result本地commit、worktree
clean；不merge/rebase/push。新Agent执行，不派子Agent。
