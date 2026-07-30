# P5-F440Z1 Router WebSocket RPC current snapshot / method table

状态：Ready。R0b2a；只升级production snapshot reader并建立immutable physical/method join。

## 直接父节点

- `P5-F440X-router-websocket-rpc-hookup-preflight-result.md`
- `P5-F440Y-router-runtime-dispatcher-websocket-rpc-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`
- `P5-F440R-router-websocket-rpc-profile-broker-core-result.md`

F440X证明Router reader仍停在旧deployment/gateway/artifact版本，并把method-bearing entry拒绝掉；current
producer已是ServiceDeployment v3、GatewayEntry v2、DeploymentArtifact v3。本leaf只负责reader hard cut与
connection capture所需snapshot结构，不接socket/broker。

实现基线为`85ff1513`对应的current integration tree。

## 目标

1. production reader只接受current identity/schema版本，删除旧v1/v2假设；
2. 把WebSocket deployment snapshot建成：

```text
physical entry (method = null, websocketConnect, rpcProfiles)
  -> optional connect handler
  -> immutable Map<external method, method binding>

method binding (method = string, websocketJsonRpc, exact profile)
  -> required private handler
  -> exact method GatewayEntryIdentity
```

3. strict join同deployment owner、host/path、protocol、physical `WebSocketEntryId`与profile；
4. 提供Gateway attach时一次性复制的immutable method table；后继连接生命周期不再读current snapshot。

本leaf不修改Gateway、RuntimeEndpoint、broker、dispatcher或server。

## 唯一写集

生产：

- `router/src/router/runtimeAssemblySnapshot.ts`
- `router/src/router/runtimeAssemblyDeploymentSnapshot.ts`
- 新建`router/src/router/runtimeAssemblyWebSocketSnapshot.ts`
- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`
- 上述reader的private type/helper

测试：

- 新建`router/tests/runtime-assembly-websocket-rpc-snapshot.test.ts`
- `router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts`
- `router/tests/compilerGeneratedManifestCompatibility.test.ts`
- 因production hard cut机械失效的以下Router direct identity call-site：
  - `router/tests/assembly-replica-dispatch.test.ts`
  - `router/tests/assembly-runtime-endpoint.test.ts`
  - `router/tests/host-ingress.test.ts`
  - `router/tests/router-default-spawn-probe.test.ts`
  - `router/tests/service-error-cross-layer-convergence.test.ts`
- 本leaf result

禁止修改Gateway/socket lifecycle/broker/RuntimeEndpoint/RuntimeDispatcher/server、Rust、cross-system fixture、
README/checker、其它task/result。不得派子Agent，不得启动server/network/live。

## Current hard cut

reader必须使用current artifact model严格解析/验证：

- ServiceDeployment v3；
- GatewayEntry v2；
- DeploymentArtifact v3；
- current GatewayEntryIdentity/Deployment identity preimage与canonical prefix；
- method selector、surface、adapter source与handler owner。

不得保留旧prefix fallback、dual reader、alias或根据字段猜版本。Router direct test中的旧literal应改成由
current test producer/helper生成或精确current identity；cross-system samples留F0。

## Physical/method join

physical：

- WebSocket protocol、method null；
- adapter kind `websocketConnect`；
- surface声明closed `rpcProfiles`；
- connect handler可有可无；
- exactly one compiler-owned physical identity。

method：

-同host/path、method non-empty；
- adapter kind `websocketJsonRpc`；
- profile必须属于physical `rpcProfiles`；
- required private current-package handler；
- exact method gateway identity；
-不得有connect-only pre/guard或成为attach route。

fail closed：

- duplicate method；
- orphan method；
-跨deployment/package/host/path/physical id；
- profile不支持；
- method无handler/错误adapter；
-两个歧义physical entry；
- method identity或surface drift。

snapshot API返回的method map应为只读/copy-on-capture结构；不得暴露mutable current snapshot引用给socket
lifecycle。

## Test-first与验证

先增加真实current method snapshot positive，使旧reader因method/prefix/version拒绝而RED。至少覆盖：

- physical有handler + methods；
- handlerless physical + methods；
- pure path-only无method；
-多个methods/profile；
-duplicate/orphan/cross-owner/profile/identity negatives；
- current v3/v2/v3 identities通过，旧版本严格失败；
- immutable map在active snapshot replacement后保持旧内容；
- filesystem loader与compiler-generated compatibility current。

必跑：

```bash
router/node_modules/.bin/vitest list --root router \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/compilerGeneratedManifestCompatibility.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/compilerGeneratedManifestCompatibility.test.ts
pnpm --dir router type-check
git diff --check
```

再运行任务写集中被hard cut触及的五个direct tests；result记录实际count，不跑完整Router suite。

## 停止与交付

若current reader需要修改artifact producer/schema，返回`TASK_SCOPE_EXPANDED`，不得兼容旧版本。若immutable
method table必须由Gateway而非snapshot拥有，可返回只读binding结构，但不能接socket。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440z1-router-rpc-snapshot`
- branch：`codex/p5-f440z1-router-rpc-snapshot`
- result：`P5-F440Z1-router-websocket-rpc-snapshot-result.md`

Implementation与result分开提交；不merge/rebase/push。
