# P5-F380 Relay interface receiver and gateway completion

状态：Ready（新节点继续clean F376 checkpoint）。

## 直接父节点

- `P5-F376-relay-http-gateway-resume-blocker.md`
- 原完整验收：`P5-F367-relay-http-gateway-migration.md`

父节点已确定compiler receiver规则无需改变：Relay具体实现已有receiver，缺口只是public instance
interface method没有显式`self: Self`。本节点授权这一个新增source owner，并完成原F367真实发布。

## Worktree与checkpoint

- `/Users/geek/workspace/internals-p5-f367-relay-http-gateway`
- branch `codex/p5-f367-relay-http-gateway`
- clean HEAD `66afed285160e2a110850ccc9407cfe49e15e86c`

不得reset/rebase或重写两个现有checkpoint。

## 必须完成

1. 在`codex-relay/service/relay.skiff`仅为
   `CodexRelayProxyClient`中缺失receiver的instance methods增加首参数`self: Self`：
   - 不改method name、其它参数、return、async/stream/effect或实现；
   - 接口与现有具体实现receiver对应；
   - 不新增或删除service-call operation。
2. 使用包含F374的最新Skiff phase-05 integration和新的fresh artifact root，重新发布：

```text
std -> llm-api -> llm-providers -> Codex Relay
```

3. 以真实Relay artifact/receipt为权威完成F367：
   - ServiceContract精确2个operation；
   - gateway entries/ingress各30；
   - 27个rawHttp unary、3个rawHttp server stream；
   - 30个method/path/key/selector唯一且reference闭合；
   - API无15个external-only scalar；
   - 无`routes`、`operation`、旧contract-root ingress；
   - canonical默认host按真实receipt断言；
   - timeout只来自`config.dev.yml = 120000`。
4. 若真实receipt暴露静态测试错误，只修改
   `codex-relay/service/service-api-receipt.test.mjs`；不得改业务proxy行为来迎合测试。

## 写入边界

允许：

- `codex-relay/service/relay.skiff`
- F367原允许的`api.yml`、`service.yml`、`service-api-receipt.test.mjs`

除已经cherry-pick的F368 package文件外，禁止其它Internals、Skiff、skiff-packages、共享scripts、
stable/live、OAuth或外部上游。

运行非零source/compiler/receipt测试、fresh真实发布和`git diff --check`。最终追加一个清晰commit，
worktree clean，不merge/rebase/push。返回完整F367自验收矩阵、exact commit/tree与receipt；主Agent据此
写F367 result并合入Internals integration。新Agent执行，不派子Agent。
