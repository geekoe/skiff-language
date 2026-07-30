# P5-F445H I7 P8 R Router 普通入口复用结果

状态：

```text
PASS
NO_PRODUCTION_CHANGE = YES
ROUTER_ORDINARY_INGRESS_REUSED = YES
DECISION_REQUIRED = NO
```

## 1. Baseline 与预检

冻结 baseline 为 Skiff
`45a89dc40dd2f4cffc19296acc9a31065fcc3a37`
（tree `e67bfc6553b9a59797b04a4722768ee765529947`）。

在创建 worktree 前完成只读审计：

- `readServiceDeploymentSelector` 已严格读取普通
  `x-skiff-service` / `x-skiff-version`，缺失、歧义或非 canonical 值均 fail closed；
- `AssemblyHttpGateway` 已用
  `service + version + HTTP method + URL path` 从当前 assembly 精确选择 ingress；
- Host 只用于构造 `httpRequest.url` metadata，不进入 ingress key；
- unary 与 raw HTTP server stream 均使用同一普通 assembly dispatch；
- server stream 已有 response writer backpressure、client disconnect cancel 和 lifecycle 清理。

结论是 self-ingress 不需要任何 test-only route、header、session、token 或 frame，也不需要修改
`router/src/**`。test-runner 只需向普通动态 business URL 发 HTTP 请求，并携带当前 case
生成 deployment 的 service/version selector。

## 2. 最小证据补充

现有测试已经覆盖：

- selector header 缺失与非法值；
- 相同 Host、相同 method/path 下按不同 service 精确分发，并把精确 deployment/generation
  写入 Runtime frame；
- Host 不参与路由；
- raw HTTP server stream 的 chunk 顺序、backpressure 与 client disconnect cancel。

审计发现缺少一条直接证明“selector 语法合法但不匹配当前 assembly 时不会 dispatch”的 HTTP
入口测试，因此只在
`router/tests/runtime-assembly-unary-dispatch.test.ts`
补充一个参数化负例。错误 service、version、method、path 均返回
`404 AssemblyIngressNotFound`，且 dispatcher 无 pending lifecycle。

生产代码、协议、配置和公共 surface 均未修改。

## 3. 验证

```text
pnpm --dir router exec vitest run \
  tests/service-deployment-selection.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/assembly-http-gateway-stream.test.ts
=> 3 files passed, 39 tests passed

pnpm --filter @skiff/router test -- \
  tests/service-deployment-selection.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/assembly-http-gateway-stream.test.ts
=> 59 files passed, 846 tests passed

pnpm --filter @skiff/router type-check
=> PASS

git diff --check
=> PASS
```

第二条命令中的 `--` 由当前 package script 传给 Vitest 后展开了完整 Router suite；因此同时留下
聚焦 39 条与完整 846 条证据。

## 4. Handoff

```text
RESULT = PASS
PRODUCTION_ROUTER_CHANGE = NONE
NEW_TEST_ONLY_PROTOCOL = NONE
UNBLOCKS = P5-F445H-I7-P8-T
```
