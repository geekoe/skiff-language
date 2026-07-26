# P5-F390 Compiler→Router typed-null ping fixture result

状态：TASK_SCOPE_EXPANDED / SAFE CHECKPOINT（fixture迁移与HTTP gateway direct join正例已完成；
fresh artifact不能通过完整`FilesystemRuntimeAssemblySnapshotLoader`，因为Router仍冻结旧identity
generation）。

## 1. Exact checkpoint

| 项目 | commit | tree |
| --- | --- | --- |
| clean base / F389文档checkpoint | `2735b2f1f2563ca13beab444007b1c065ffdf01c` | `478a0c684de2b312958051de4103a4cde9f119d4` |
| fixture与direct tests | `53c79dc6e029137d7e0ba987f8ba5f5fb0de480f` | `108887768ed89d9d0923f933bfb5708bfa1a4c74` |

工作分支为`codex/p5-f389-compiler-router-http-fixture`，worktree为
`/Users/geek/workspace/skiff-p5-f389-compiler-router-http-fixture`。本任务没有merge/rebase、push、
stable/live或instance操作。

## 2. 已完成的fixture终态

- `main.ping() -> string`实现逐值保持不变，仍返回`"pong"`。
- 新增private `main.__skiffHttpPing(body: null) -> string`；函数体只调用`ping()`并返回结果。
- `api.yml`变为`{}`；Package public symbols、service-call roots、ServiceContract operations与
  deployment operation bindings均为零。wrapper和`ping`仍只存在于implementation symbols。
- `service.yml`中的旧`http.routes[].operation`已替换为唯一named entry `http.ping`：
  - selector仍精确为`websocket-fixture.skiff.localhost`、`GET`、`/ping`；
  - `kind: typedJson`、handler `main.__skiffHttpPing`；
  - 唯一adapter arg为`body <- http.body`。
- 未修改compiler/Router production、runtime、Host、test-runner、其它fixture或任何WebSocket
  production/test。

## 3. Fresh publish/build-only事实

真实`package publish`和`assembly build`使用新的临时artifact root完成。关键结果为：

| 对象 | fresh事实 |
| --- | --- |
| PackageArtifact | schema `v7`；build `skiff-package-build-v8:sha256:8473eab7ab1e8bc914fcd3256473d70453329a0e9251d4ac955da59cdf57fad6`；`serviceCallRoots = []`；public symbols `0`；implementation symbols精确为`main.__skiffHttpPing`与`main.ping` |
| ServiceContract | schema `v4`；protocol `skiff-service-protocol-v4:sha256:998f616d457989ef92c0b8afd37e09e6d8e3d7ce60c3ecefc64c1c99aa100819`；operations `0` |
| ServiceDeployment | schema `v2`；operation bindings `0`；gateway entries `1`；ingress `1` |
| gateway `ping` | identity `skiff-gateway-entry-v1:sha256:adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653`；exact private handler；`typedJson + unary`；external sources只含`http.body`；request Null；response String |
| RuntimeAssembly | schema `v2`；identity `skiff-runtime-assembly-v2:sha256:e02ffe353e054dfd4167ca89524d37b68c80d8e046b61b46fe42e8cbe04610e2`；root/deployment/contract/package/gateway ingress各`1` |

assembly中的deployment ref、package ref、gateway key、gateway identity与selector均和fresh
deployment逐值相等。四条`jq -e`断言分别验证Package、Contract、Deployment和Assembly，均返回
`true`。

## 4. Direct Router证据的精确边界

`router/tests/compilerGeneratedManifestCompatibility.test.ts`现在：

1. 从fresh compiler publish/build读取真实Package/Contract/Deployment/Assembly records；
2. 精确断言上述0-op/1-gateway、wrapper signature、Null/String schema、identity、selector与引用闭合；
3. 为了只验证HTTP gateway/deployment join，test-local clone仅把deployment contract identity的
   `skiff-service-protocol-v4:`词法前缀换成Router当前接受的`v3`，随后由production
   `joinRuntimeAssemblyDeployments`成功解码并得到一个`typedJson + unary` gateway binding；
4. 未修改的exact fresh artifact继续明确断言完整
   `FilesystemRuntimeAssemblySnapshotLoader`因protocol generation skew而fail closed。

因此本结果只声明**HTTP gateway direct join seam正例**，不声明完整filesystem loader compatibility。
`dynamic-build-id-parity.test.ts`保留compiler-authored canonical record path正例，并把完整loader当前
version skew冻结为显式fail-closed证据。

## 5. 完整loader blocker

fresh compiler与Router production的generation精确不一致：

| 边界 | fresh compiler | Router owner |
| --- | --- | --- |
| ServiceProtocolIdentity | `skiff-service-protocol-v4` | `router/src/router/runtimeAssemblySnapshot.ts:12-13`和`runtimeAssemblyDeploymentSnapshot.ts:17-18`只接受`v3` |
| PackageBuildId | `skiff-package-build-v8` | `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts:91-95`只接受`v4` |
| FileIrIdentity | `skiff-file-ir-v8` | 同文件`:107-111`只接受`v5` |

exact full loader的首个错误为：

```text
RuntimeAssembly.resolvedContracts[0].serviceProtocolIdentity is invalid
```

exact deployment direct join在未做test-local隔离时同样先返回：

```text
RouterSnapshot.serviceDeployments[0].contract.serviceProtocolIdentity is invalid
```

解除这两个v3 lexical gate后，`loadActorMethods`会依次在当前`v8` PackageBuildId对`v4`门禁及当前
`v8` FileIrIdentity对`v5`门禁失败。这些owner属于Router current artifact generation消费，不属于
typed-null fixture；F390明确禁止修改它们。

## 6. 最小后继范围

一个独立Router filesystem/current-generation任务应：

1. 原子收敛`runtimeAssemblySnapshot.ts`与`runtimeAssemblyDeploymentSnapshot.ts`的contract ref到
   current `skiff-service-protocol-v4`，不增加v3/v4 dual-read；
2. 把`filesystemRuntimeAssemblySnapshotLoader.ts`的PackageBuild/FileIr path identity owner收敛到
   current `v8/v8`，并按PackageArtifact v7 / File IR v8字段复核actor catalog读取；
3. 用未改写的fresh compiler records使
   `compilerGeneratedManifestCompatibility.test.ts`和
   `dynamic-build-id-parity.test.ts`恢复完整filesystem loader正例，删除本checkpoint的test-local
   contract prefix隔离及version-skew负例；
4. 聚焦重跑上述两个direct tests、
   `filesystem-runtime-assembly-snapshot-loader.test.ts`和Router targeted type-check。

该后继不需要改compiler output、F390 fixture、HTTP gateway语义或WebSocket协议。

## 7. 验证

| 命令 | 结果 |
| --- | --- |
| fresh `skiff-compiler package publish ... --json` | PASS；真实Package/Contract/Deployment |
| fresh `skiff-compiler assembly build ... --json` | PASS；真实RuntimeAssembly v2 |
| 四条Package/Contract/Deployment/Assembly `jq -e`闭合断言 | PASS；四个`true` |
| `pnpm test:manifest-compatibility` | PASS；1/1，非零 |
| `pnpm exec vitest run tests/dynamic-build-id-parity.test.ts` | PASS；4/4，非零 |
| direct test targeted `tsc --noEmit ...` | PASS |
| `pnpm type-check` | BASE BLOCKED；只命中既有WebSocket/legacy assembly类型不收敛，首个位置为`src/gateway/assemblyWebSocketGateway.ts:254`；未修改这些文件 |
| fixture反搜`operation/routes/serviceCall/contractOperationId` | PASS；零匹配 |
| production diff check（compiler/input/driver、artifact-model、Router src、runtime、test-runner） | PASS；零diff |
| `git diff --check` | PASS |
