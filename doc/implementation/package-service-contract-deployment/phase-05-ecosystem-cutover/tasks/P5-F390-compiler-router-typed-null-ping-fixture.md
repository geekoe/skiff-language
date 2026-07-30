# P5-F390 Compiler→Router typed-null ping fixture

状态：Ready（修订F389矛盾合同）。

## 直接父节点

- `P5-F389-compiler-router-http-fixture-migration-blocker.md`
- `P5-F357-http-gateway-compiler-projection-result.md`

## Worktree

- `/Users/geek/workspace/skiff-p5-f389-compiler-router-http-fixture`
- branch `codex/p5-f389-compiler-router-http-fixture`
- clean base；F389没有production改动。

## 必须完成

1. 保持原`ping() -> string`实现及HTTP method/path/host逐值不变。
2. 新增private `__skiffHttpPing(body: null) -> string` wrapper，只调用`ping()`并返回结果；wrapper不进入
   `api.yml`。
3. Package API移除external-only `ping` projection，使ServiceContract `1 -> 0`。
4. 旧route改为一个named gateway entry：
   - `kind: typedJson`、unary；
   - `body <- http.body`；
   - request schema Null、response schema String；
   - handler指向exact private wrapper。
5. fresh compile/publish/build-only证明0 roots/operations/bindings、1 gateway/ingress、selector/key/handler/
   identity/mode/reference闭合。
6. 更新direct compiler→Router generated artifact compatibility断言；真正WebSocket断言保持，不修改WS
   production或协议。

允许写fixture目录和其direct compiler/Router测试。禁止compiler/Router production、其它fixture、
test-runner、stable/live。运行非零直接测试、fresh artifact、旧HTTP operation反搜和diff check。

写`P5-F390-compiler-router-typed-null-ping-fixture-result.md`，本地commit、worktree clean；不
merge/rebase/push。新Agent执行，不派子Agent。
