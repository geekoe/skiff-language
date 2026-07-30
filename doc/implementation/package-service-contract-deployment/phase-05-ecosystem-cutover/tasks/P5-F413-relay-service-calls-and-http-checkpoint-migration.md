# P5-F413 Codex Relay serviceCalls and HTTP checkpoint migration

状态：Ready。

## 直接父节点

- `P5-F403-service-calls-manifest-implementation-audit-result.md`
- `P5-F409-typed-service-selection-contract-driver-result.md`
- `P5-F367-relay-http-gateway-migration.md`

F367 的 Relay worktree 已形成 clean checkpoint：

```text
/Users/geek/workspace/internals-p5-f367-relay-http-gateway
68c7d679...
```

它包含所需的 Relay 30 个 named rawHttp entry、API 收缩和 public-instance receiver 修复，但该分支的
祖先还带有 Account 改动，并使用已废弃的 `api.yml serviceCall` 选择。不得整分支合并；本节点从最新
Internals integration 新建 worktree，只移植 Relay-owned hunks并适配新模型。

## 精确代码状态与写入范围

- Internals start：
  `3a7234610c53b11c5f2cfdb5b04448408e924e31`。
- Skiff toolchain：
  `/Users/geek/workspace/skiff-phase-05-integration`，执行时记录 exact commit/tree。
- 只允许修改：

```text
codex-relay/service/**
```

不得修改 Account、AIHub、Agine、共享 scripts、Skiff、skiff-packages、stable/live 或设计。

## 必须实现

1. 从 F367 checkpoint 只移植 Relay 文件中仍有效的 30 个 named `rawHttp` entries：
   - method/path、handler、adapter source 与 stream mode 保持 checkpoint 的精确值；
   - 删除 legacy `routes`、每条 route 的 `operation` 与 service-level duplicate timeout；
   - 27 个 raw unary、3 个 raw server-stream；
   - 外部 HTTP entries 不进入 ServiceContract。
2. Package API 只保留 Relay 对外 package surface；`relayProxy` 必须使用：

```yaml
relayProxy:
  const: relay.relayProxy
  interfaces:
    - relay.CodexRelayProxyClient
```

   不得保留 `serviceCall` marker。若 checkpoint 中 receiver 声明修复仍缺失，应只移植相应 Relay
   `.skiff` hunk。
3. `service.yml` 增加：

```yaml
serviceCalls:
  - relayProxy
```

   生成 ServiceContract 必须精确只有：
   - `relayProxy.responsesCompleted`
   - `relayProxy.responsesCompletedResult`
4. 更新 Relay source/receipt test：
   - protocol v4、operation ID v1；
   - 精确 2 个 service operations、30 个 gateway entries；
   - 30 个 selector 唯一且 HTTP 外部入口不要求出现在 service operations；
   - API const/interfaces 与 manifest selection 精确；
   - 不保留旧 marker、route-operation 或旧协议断言。
5. 保持上游流 wire 与业务实现不变；不得把 raw server stream 改成 typed JSON、service stream或聚合响应。

## 验证与交付

先检查 repo scripts，运行现有 Relay service API 聚焦 test；至少：

```bash
node --test codex-relay/service/service-api-receipt.test.mjs
git diff --check
```

若已有 isolated receipt owner，可设置 `SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration` 运行，但不得
操作 stable/live、真实 OAuth 或外部上游。若真实 authoring 被本任务外共享 ingress owner 阻塞，保留静态
聚焦证据并精确报告；不得扩张。

生产与测试改动提交为一个 clean commit；返回 exact base、commit/tree、changed files、测试计数与
checkpoint→当前模型的 hunk 映射。result 由主 Agent 写入 Skiff integration；不 merge/rebase/push，不得
派子 Agent。
