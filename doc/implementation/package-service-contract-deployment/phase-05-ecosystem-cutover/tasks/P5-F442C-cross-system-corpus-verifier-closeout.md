# P5-F442C Cross-system corpus / verifier closeout

状态：Ready。刷新current external authoring与generic request corpus，关闭cheap combined的X节点。

## 直接父节点

- `P5-F442A-final-fixture-tooling-preflight-result.md`

父审计确认：

- generic legacy request branch仍是current transport negative owner，但其外层
  ServiceProtocol必须为v5；
- checkpoint/verifier仍把HTTP/WebSocket/timeout错误地归给`service.yml`；
- obsolete `runtime-websocket-response-wire.json`无consumer，含旧receive positive与旧identity；
- connect corpus中的named receive rejection mutations是current negative，必须保留。

实现基线为 `0303fe5d`。

## 写集与要求

只允许修改：

- `cross-system-fixtures/package-service-ecosystem/checkpoint.json`
- `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`
- `cross-system-fixtures/package-service-ecosystem/verify.mjs`
- 删除
  `cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json`
- 本节点result

要求：

1. checkpoint/assertions表达current external authoring：
   - `service.yml`拥有service id、`serviceCalls`，test service可有`kind:test`；
   - `http.yml`顶层是named entry mapping；
   - `websocket.yml`是唯一physical entry，含path、可选connect、可选jsonRpc；
   - timeout来自profile config，不在service.yml；
2. generic `legacyRequestStartHeaders`保留，但ServiceProtocol v3刷新为v5，使测试到达
   “legacy-only”目标；
3. 删除无consumer的obsolete response corpus；
4. 不删除current connect corpus中的`receiveEvent`/old adapter rejection；
5. 不修改Router/Rust production，不新增兼容读取。

## Test-first与验证

先执行并记录至少一个父审计真实失败。worktree无依赖时可临时链接已有依赖，但交付前必须删除。

```bash
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
node cross-system-fixtures/package-service-ecosystem/verify.mjs --combined-probe
node cross-system-fixtures/package-service-ecosystem/verify.mjs \
  --runtime-wire-self-test
router/node_modules/.bin/vitest list --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-protocol-websocket-response.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-protocol-websocket-response.test.ts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-transport runtime_assembly_request
git diff --check
```

记录non-zero Router count。不得启动stable、network、live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f442c-cross-system-corpus`
- branch：`codex/p5-f442c-cross-system-corpus`
- result：`P5-F442C-cross-system-corpus-verifier-closeout-result.md`

Implementation与result分开提交。5分钟内开始实际fixture修改；不得派子Agent，不得
merge/rebase/push。
