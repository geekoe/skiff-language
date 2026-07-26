# P5-F392 Router current artifact generations result

状态：Completed。Router RuntimeAssembly contract/deployment decode与filesystem actor catalog loader
已原子切到current `v4/v8/v8`，F390 fresh compiler records现在不经任何词法改写即可同时通过direct
deployment join和完整filesystem loader。

## 1. Exact checkpoint与边界

| 项目 | commit | tree |
| --- | --- | --- |
| task base | `0674c6bfe833c9d3e13dad3b9e26e41e7d3a4e07` | `107629a2764664affa0eea663662637fb3dee08b` |
| task delivery | 本result所在本地commit | 本result所在本地commit的tree |

工作分支为`codex/p5-f392-router-current-artifact-generations`，worktree为
`/Users/geek/workspace/skiff-p5-f392-router-current-artifact-generations`。

本任务只修改三个Router owner、它们的direct tests、F390两个compatibility tests和本result。
没有修改compiler output、artifact schema/identity、F390 fixture、HTTP/WS gateway语义、
Host/runtime/test-runner、其它仓库或stable/live状态；没有merge、rebase或push。

## 2. Current generation终态

- `runtimeAssemblySnapshot.ts`和`runtimeAssemblyDeploymentSnapshot.ts`的
  `ServiceProtocolIdentity` lexical gate都只接受
  `skiff-service-protocol-v4:sha256:<64 lowercase hex>`；production owner中没有v3 dual-read。
- `filesystemRuntimeAssemblySnapshotLoader.ts`从
  `RuntimeAssembly.packageLinkPlan.codeSlots[].package`读取implementation package
  coordinate/version/build identity，只接受`skiff-package-build-v8`并按canonical
  coordinate/version/build hash读取`PackageArtifact` record。
- PackageArtifact v7仍以`files[].fileIrIdentity/modulePath`携带文件引用；loader只接受
  `skiff-file-ir-v8`并从同一package build目录下的`file-ir/<hash>.json`读取record。
- FileIR v8仍以`actorDeclarations[]`、`abi.actorName`、
  `actorAbiIdentity`、`actorImplementationIdentity`和
  `methodImplementations`表达actor catalog。现有actor method projection与current字段逐项一致，
  因此除identity generation外不需要相邻协议字段变化；本任务没有新增unknown
  passthrough或fallback。

current records不要求共享协议变化，本任务没有触发`TASK_SCOPE_EXPANDED`。

## 3. Exact fresh record tests

`compilerGeneratedManifestCompatibility.test.ts`保留F390 real publish/build helper产生的原始records：

- PackageArtifact精确为schema v7 / build v8 / FileIR v8；
- ServiceContract精确为schema v4 / protocol v4；
- ServiceDeployment精确为schema v2、0 operation bindings和1个typed-null HTTP gateway；
- RuntimeAssembly精确为schema v2、1 deployment / contract / package / gateway ingress。

direct `joinRuntimeAssemblyDeployments`现在直接消费`generated.deploymentValue`。原test-local
v4→v3 `replace`、clone隔离注释和“current version skew必须失败”的负例均已删除。随后完整
`FilesystemRuntimeAssemblySnapshotLoader`读取同一未修改artifact root，结果与direct join逐值相等。

`dynamic-build-id-parity.test.ts`继续验证compiler-authored canonical record path及escape负例，并把
完整loader断言改为exact current record正例。filesystem owner的synthetic direct fixture也已切到
protocol v4和package build v8。

## 4. Verification

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| focused non-zero tests | `pnpm --filter @skiff/router exec vitest run tests/compilerGeneratedManifestCompatibility.test.ts tests/dynamic-build-id-parity.test.ts tests/filesystem-runtime-assembly-snapshot-loader.test.ts` | PASS；3 files / 30 tests |
| scoped type-check | `pnpm --dir router exec tsc --noEmit --pretty false --target ES2022 --lib ES2022 --module NodeNext --moduleResolution NodeNext --strict --noUncheckedIndexedAccess --exactOptionalPropertyTypes --esModuleInterop --skipLibCheck --forceConsistentCasingInFileNames <三个owner与三个direct tests>` | PASS |
| Router type-check | `pnpm --filter @skiff/router exec tsc --noEmit --pretty false` | 预期非零；44个错误全部为既有WS/HTTP-only残留；本任务owner与direct tests零错误 |
| production旧generation反搜 | 三个owner内搜索`service-protocol-v3\|package-build-v4\|file-ir-v5` | PASS；零匹配 |
| direct-test compatibility反搜 | 三个direct/compatibility tests内搜索旧generation、version-skew文案及`.replace(` | PASS；零匹配 |
| patch hygiene | `git diff --check` | PASS |

Router全局type-check残留精确分类如下：

| 范围外owner | errors | 分类 |
| --- | ---: | --- |
| `src/gateway/assemblyWebSocketGateway.ts` | 8 | 旧WebSocket binding/request DTO consumer |
| `src/router/assemblyRuntimeRegistry.ts` | 4 | 旧WebSocket ingress identity helper |
| `tests/assembly-websocket-gateway.test.ts` | 19 | 旧WebSocket request DTO fixture |
| `tests/loop-risk-health.test.ts` | 1 | legacy/canonical request header union |
| `tests/router-websocket-trust-dispatch.test.ts` | 5 | 旧WebSocket selector/caller fixture |
| `tests/runtime-endpoint-connection-send-trust.test.ts` | 5 | 旧WebSocket binding/contract fixture |
| `tests/service-error-cross-layer-convergence.test.ts` | 2 | HTTP/WebSocket混合旧selector fixture |
| 合计 | 44 | 本任务禁止越界修复 |

依赖预检首次发现worktree没有安装Vitest，因此该次命令在测试收集前以
`Command "vitest" not found`退出，不计为测试证据。随后使用checked-in lockfile执行
`pnpm --dir router install --frozen-lockfile`，没有修改tracked dependency文件，再原样运行并得到上述
3 files / 30 tests的有效证据。

## 5. 自验收矩阵

| 任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| ServiceProtocol current v4 only | `runtimeAssemblySnapshot.ts:12-13`；`runtimeAssemblyDeploymentSnapshot.ts:17-18` | 两owner无`service-protocol-v3` | exact direct join与full loader正例；30/30 |
| PackageBuild/FileIR current v8/v8 only | `filesystemRuntimeAssemblySnapshotLoader.ts:91-116` | loader无`package-build-v4`/`file-ir-v5` | fresh compiler package/file records通过full loader |
| current actor/implementation/path shape | `loadActorMethods`按code slot package ref、PackageArtifact v7 files和FileIR v8 actor declarations读取 | 无fallback、dual path或unknown generation acceptance | fresh canonical package/file path正例；escape负例保留 |
| 删除F390 compatibility隔离 | `compilerGeneratedManifestCompatibility.test.ts:158-188`；`dynamic-build-id-parity.test.ts:107-125` | 无`.replace(`、v3或version-skew负例 | direct join与full loader都消费exact fresh records |
| owner类型与范围闭合 | 三个production owner及三个direct tests | production旧generation零匹配；范围外WS残留单独分类 | scoped `tsc` PASS；global仅既有44 errors；`git diff --check` PASS |
