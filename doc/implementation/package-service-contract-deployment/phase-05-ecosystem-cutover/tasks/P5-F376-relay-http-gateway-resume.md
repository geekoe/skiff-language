# P5-F376 Codex Relay HTTP gateway resume

状态：Ready（恢复同一未完成F367任务；使用新Agent）。

## 直接父节点

- 原任务：`P5-F367-relay-http-gateway-migration.md`
- 已完成编译器前置：`P5-F374-package-signature-exact-symbol-owner-result.md`
- 已完成Host前置：`P5-F365-host-http-gateway-admission-wire-result.md`
- Internals package前置：`P5-F368-internals-error-payload-marker-cleanup.md`

父节点已经关闭真实发布曾遇到的两个前置：`llm-api`的过时`ErrorPayload` marker和
`std.http.stream` public signature slot 7 owner歧义。本节点只恢复并完成F367，不扩大Relay业务范围。

## 保留状态

- worktree：`/Users/geek/workspace/internals-p5-f367-relay-http-gateway`
- branch：`codex/p5-f367-relay-http-gateway`
- 当前保留三个未提交owned文件：
  - `codex-relay/service/api.yml`
  - `codex-relay/service/service.yml`
  - `codex-relay/service/service-api-receipt.test.mjs`
- 已有静态聚焦测试为4/4；这是checkpoint证据，不代替真实发布。

不得checkout、reset、stash或丢弃现有diff，不得从头重写。

## 恢复步骤

1. 开始时核对dirty inventory恰为上述三个文件，运行`git diff --check`，审阅其是否仍满足原F367
   `17 -> 2`及30个HTTP entry要求。
2. 先把现有owned diff提交为明确的本地checkpoint；checkpoint不是完成声明。
3. cherry-pick Internals前置production commit
   `4ebdb5784de672943a5917b1beb43bf26d64db82`。若发生真实冲突，停止并报告；不得手工复制或重写
   package错误语义。
4. 使用包含F374的`/Users/geek/workspace/skiff-phase-05-integration`和fresh temporary artifact root：
   - bootstrap canonical std；
   - 依序真实发布`packages/llm-api`、`packages/llm-providers`；
   - 真实发布Codex Relay service package；
   - 不读取stable artifact store。
5. 以真实artifact/receipt为权威修正Relay局部receipt测试；不得保留仅适配旧合成fixture的断言。

## 最终不变量

- Package API不再包含15个external-only scalar export。
- ServiceContract精确2个ordinary service-call operation：
  - `relayProxy.responsesCompleted`
  - `relayProxy.responsesCompletedResult`
- gateway entries与ingress各30：
  - 27个`rawHttp` unary admin/OPTIONS entry；
  - 3个`rawHttp` server-stream `/v1` entry；
  - method/path逐值保持，selector/key唯一；
  - 每项只有`request <- http.request`；
  - 无`routes`、`operation`或旧contract-root ingress。
- canonical默认host以真实receipt为准；若authoring默认值是`"*"`，测试不得继续写合成的空字符串。
- timeout只由现有`config.dev.yml`提供，值仍为`120000`。
- 原始OpenAI-compatible raw request与`HttpResponseStreamEvent`顺序流语义不变。

## 写入与验证

Relay写入范围仍严格沿用F367；除精确cherry-pick的F368 package文件外，不修改其它Internals production。

```bash
node --test codex-relay/service/service-api-receipt.test.mjs
git diff --check
```

记录非零测试、fresh std/llm-api/llm-providers/Relay receipt、2/30/30计数、27 unary/3 server-stream模式及
identity generation。完成时：

- checkpoint之后可有一个Relay receipt修正commit；
- worktree clean，不merge/rebase/push，不操作stable/live或外部上游；
- 返回完整F367自验收矩阵和exact commit/tree，由主Agent写F367 result并集成清理。

新Agent执行，不复用原会话，不派子Agent。若真实发布仍暴露新的共享owner，按工作流返回
`TASK_SCOPE_EXPANDED`。
