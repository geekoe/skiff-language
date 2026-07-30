# P5-F414 AIHub serviceCalls selection

状态：Ready。

## 直接父节点

- `P5-F403-service-calls-manifest-implementation-audit-result.md`
- `P5-F409-typed-service-selection-contract-driver-result.md`

AIHub 当前 Package API 已声明两个 public instances；本节点只把其内部服务调用面从“整个公开 API
自动成为 operations”的旧测试假设，迁到 `service.yml.serviceCalls` 的显式选择。外部
HTTP/WebSocket authoring 仍使用既有 shape，本节点不决定或迁移其模型。

## 精确代码状态与写入范围

- Internals start：
  `3a7234610c53b11c5f2cfdb5b04448408e924e31`。
- Skiff toolchain：
  `/Users/geek/workspace/skiff-phase-05-integration`，执行时记录 exact commit/tree。
- 只允许修改：

```text
aihub/service/api.yml
aihub/service/service.yml
aihub/service/service-api-receipt.mjs
aihub/service/service-api-receipt.test.mjs
```

不得修改 AIHub `.skiff` 实现、HTTP/WebSocket route shape、Agine/Relay/Account、共享 scripts、
Skiff/skiff-packages、stable/live 或设计。

## 必须实现

1. `api.yml` 保持完整 Package public graph，两个实例必须继续是完整
   `const + interfaces` block；不得删除 external helper exports，也不得加入 marker。
2. `service.yml` 增加且只增加以下 selection：

```yaml
serviceCalls:
  - managedLlm
  - providerCatalog
```

3. generated Service API oracle 精确为 5 个 public-instance methods：
   - `managedLlm.validateChat`
   - `managedLlm.streamChat`
   - `managedLlm.webSearch`
   - `providerCatalog.builtinProvider`
   - `providerCatalog.model`
4. `handleAihubHttp`、`selectProvider`、`websocket` 是 Package public helpers，但未被 service manifest
   选择，必须不进入 ServiceContract。接口声明名也不能变成 executable operation。
5. receipt 使用 protocol v4 与 operation ID v1；missing/extra operation 继续 fail closed。
6. 现有 package/service/config ownership 断言不弱化；额外断言 API blocks 与 selection 精确。

## 验证与交付

运行并记录非零计数：

```bash
cd aihub/service
npm run test:service-api
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration npm run type-check
git diff --check
```

如果 type-check 被本节点外的 legacy ingress/shared tooling 阻塞，记录精确错误；不得改 route shape 或扩大
范围。不得操作 stable/live 或外部服务，不得派子 Agent。

生产与测试改动提交为一个 clean commit；返回 exact base、commit/tree、changed files、测试计数与 receipt
期望。result 由主 Agent 写入 Skiff integration；不 merge/rebase/push。
