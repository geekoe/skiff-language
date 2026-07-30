# P5-F375 Registry generation revalidation result

状态：**TASK_SCOPE_EXPANDED / FAIL**。Fresh authoring成功，但真实生成的Registry
ServiceContract是`0`个operation而不是父节点冻结的`20`个；Registry production仍只接受旧generation
identity。规定的隔离runtime测试又被当前Skiff Router的production import/export断链阻塞，完整
`npm test`另被OpenAI production类型错误阻塞。本节点没有修改任何production source、consumer、
stable或live状态。

## 1. Exact checkpoints与隔离边界

| 项目 | commit | tree |
| --- | --- | --- |
| Skiff task worktree | `c893ba9e5e8497b92b8f1847bd42f257f828f33a` | `eed3a97aa124b8cc020db3390c03f416d4c81a44` |
| Skiff phase-05 integration toolchain | `c893ba9e5e8497b92b8f1847bd42f257f828f33a` | `eed3a97aa124b8cc020db3390c03f416d4c81a44` |
| skiff-packages consumer checkpoint | `0ab4e7628b0a6aa90961c1485d2e58634b902676` | `5abb824e560778fd38a0a9a4e9936d189cc9f843` |

- result worktree/branch：
  `/Users/geek/workspace/skiff-p5-f375-registry-generation-revalidation` /
  `codex/p5-f375-registry-generation-revalidation`；
- consumer：
  `/Users/geek/workspace/skiff-packages-phase-05-integration`，验证前后均clean，只读；
- manual fresh store：
  `/tmp/skiff-p5-f375-registry.LXjnrK/artifacts`，由本次任务新建；先bootstrap canonical std，再发布
  Registry，没有复用旧receipt；证据提取后整个task-owned temp root已移到系统废纸篓，可恢复；
- required npm命令显式使用`SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration`，该worktree与
  task worktree的commit/tree bit-identical；两条命令完成后，共享integration才在
  `2026-07-26 17:18:51 +0800`开始前进，并最终到
  `a708c5abc41d285a8eb4c70734ebbb0f2f2efdee` /
  `4a96ca557097d32ca85b8d5ad7e53432a0bf269c`，本文不把旧证据声明为该后续状态的证据；
- 没有读取、修改或请求stable `4000/4001`、stable artifact root、watch registry或live target。
  唯一启动的test instance使用动态HTTP/control/Mongo端口`46398/46399/46401`，失败后进程与临时目录均
  已由isolated owner清理；对应PID和端口复查均无存活listener。

## 2. Fresh publish与四对象receipt

先执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f375-registry-generation-revalidation/build/cargo-target \
cargo run --locked --quiet --manifest-path test-runner/Cargo.toml \
  --bin skiff-package-service-smoke-fixture -- \
  --bootstrap-only \
  --artifact-root /tmp/skiff-p5-f375-registry.LXjnrK/artifacts \
  --environment skiff-packages-test \
  --platform-source-root /Users/geek/workspace/skiff-p5-f375-registry-generation-revalidation

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f375-registry-generation-revalidation/build/cargo-target \
node scripts/skiff.mjs package publish \
  /Users/geek/workspace/skiff-packages-phase-05-integration/registry \
  --artifact-root /tmp/skiff-p5-f375-registry.LXjnrK/artifacts \
  --environment dev --json

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f375-registry-generation-revalidation/build/cargo-target \
node scripts/skiff-dev-sync.mjs \
  --root /Users/geek/workspace/skiff-packages-phase-05-integration/registry \
  --config /tmp/skiff-p5-f375-registry.LXjnrK/watch.json \
  --artifact-root /tmp/skiff-p5-f375-registry.LXjnrK/artifacts \
  --environment dev --build-only --json
```

`--build-only`只在fresh store中从刚发布的Registry deployment形成assembly receipt，没有activation。
实际receipt为：

| 对象 | fresh identity / revision |
| --- | --- |
| canonical std bootstrap assembly | `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f` |
| Registry PackageArtifact build | `skiff-package-build-v8:sha256:ddd228af53c6dbae3650d5256a59e340fa4c74387263dcd72c43c650a0f896fc` |
| Registry Package Local ABI | `skiff-package-local-abi-v6:sha256:c95b1889e2044c5780969ec040cfa9ec2c91afe9ba69a0c96574b733b4e71d73` |
| Registry ServiceContract | `skiff-service-protocol-v4:sha256:4257e4ae555bb6032b388d0e252f0fb7f50504b9322f0caf69518338bd5495db` |
| Registry deployment revision | `sha256-e12c0482ba8e837b1af96309aac03988f3c3c8804af58c5b51b1d49a1483d28e` |
| Registry ServiceDeployment | `skiff-deployment-artifact-v2:sha256:239e5f3fca862681457739dea5c6703d6ad076581b586c825c189c69c32c241a` |
| Registry-only RuntimeAssembly | `skiff-runtime-assembly-v2:sha256:62f284af0b2f37eaa26e0da8f61910a5e3fb697fc195023475b6471a0139af59` |

Fresh records均存在于对应canonical
`records/package-artifacts`、`records/service-contracts`、`records/service-deployments`和
`records/runtime-assemblies`路径。Package、contract和deployment publish同时写出了typed pointer
receipt；assembly这里只做immutable build，不移动environment pointer。

## 3. Blocking finding A：Registry真实contract为`20 -> 0`

`registry/api.yml:40-62`仍有父节点列出的20个source export，四类各五个：

- `packageArtifact{Put,Read,PointerRead,PointerCas,PointerHistory}`；
- `serviceContract{Put,Read,PointerRead,PointerCas,PointerHistory}`；
- `serviceDeployment{Put,Read,PointerRead,PointerCas,PointerHistory}`；
- `runtimeAssembly{Put,Read,PointerRead,PointerCas,PointerHistory}`。

Source条目计数为20，但它们全部还是scalar package-public binding。Production `api.yml`中
`serviceCall: true`命中为零。Fresh publish的实际可观察结果为：

```text
Service API for skiff.run/registry
Available: 0
Package-only: 20
```

JSON receipt的20个function虽然都有package visibility `status: "available"`，但全部缺少
`serviceOperationId`。对应fresh record进一步证明：

| record surface | 实际值 |
| --- | ---: |
| `PackageArtifact.serviceCallRoots` | `0` |
| `ServiceContract.operations` | `0` |
| `ServiceDeployment.operationBindings` | `0` |

因此source-shape测试“有20个export”不能证明20个ordinary service-call operation。当前显式
service-call选择规则要求每个function leaf使用`source + serviceCall: true`；修复属于
skiff-packages Registry production owner，超出本result-only验证节点。

Ingress隔离本身符合预期：

| generated surface | 实际值 |
| --- | ---: |
| `ServiceDeployment.gatewayEntries` | `0` |
| `ServiceDeployment.ingress` | `0` |
| `RuntimeAssembly.gatewayIngress` | `0` |

## 4. Blocking finding B：Registry仍冻结旧identity generation

Fresh records与Registry当前接受的摘要generation不一致：

| 摘要 | fresh canonical generation | Registry production / test generation |
| --- | --- | --- |
| PackageArtifact | schema `v7`；build `v8`；local ABI `v6` | schema `v2`；build `v4`；local ABI `v3` |
| ServiceContract | schema/protocol `v4` | schema/protocol `v2` |
| ServiceDeployment | schema/artifact `v2` | schema/artifact `v1` |
| RuntimeAssembly | schema/identity `v2` | schema/identity `v1` |

Production证据是`registry/immutable_store.skiff:84-91,166-172,240-247,322-326`；
`tests/registry/immutable_store.test.skiff:83-123`也只构造旧generation值。即使Router先恢复，当前
`packageArtifactPut`、`serviceContractPut`、`serviceDeploymentPut`和`runtimeAssemblyPut`仍会在
schema/prefix校验处拒绝本次fresh receipt摘要，不能满足“新版identity摘要immutable put/read”。

Pointer动态覆盖也不足：

- `tests/registry/pointer_store.test.skiff:3-50`只对PackageArtifact完成成功的两次CAS、read和ascending
  history；
- ServiceContract只有非法history limit负例；
- RuntimeAssembly只有candidate release不匹配负例；
- ServiceDeployment没有pointer调用。

因此四类record各自成功CAS/read/history的任务条款没有现成非零证据。现有Registry runtime test共有5个
test case，但本次动态执行在Router readiness前终止，实际执行数为0。

## 5. Blocking finding C：隔离Router无法启动

规定命令：

```bash
cd /Users/geek/workspace/skiff-packages-phase-05-integration
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
npm run test:registry
```

结果：**FAIL**。

- Node source tests：`5/5` PASS；
- fresh std bootstrap和Registry publish：PASS，但publish再次输出`Available: 0`、
  `Package-only: 20`；
- Registry `.test.skiff`：发现5个，执行`0`个；
- isolated Mongo与runtime曾在动态端口启动，Router每次启动立即以同一ESM错误退出，120秒readiness
  fail closed后isolated owner关闭全部组件。

精确断链：

```text
router/src/gateway/assemblyWebSocketGateway.ts:19-21
  imports canonicalAssemblyWebSocketIngressIdentity
  from ../router/assemblyRuntimeRegistry.js

router/src/router/assemblyRuntimeRegistry.ts
  does not export or define that symbol

SyntaxError: The requested module ... does not provide an export named
'canonicalAssemblyWebSocketIngressIdentity'
```

这是Skiff Router production owner，不是Registry consumer可以修复的fixture问题。本任务不得修改Skiff
production来穿过该blocker，也不得改用stable instance补证据。

## 6. Required full package gate

规定命令：

```bash
cd /Users/geek/workspace/skiff-packages-phase-05-integration
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
npm test
```

结果：**FAIL；0个runtime test执行**。Fresh std bootstrap以及`http-session`、`aliyunoss` package
publish成功；在任何test service启动前，`openai` package publish失败：

```text
openai/openai.skiff:465:62
openai/openai.skiff:471:58
readImageResponse argument 3 canonical type identity/type mismatch
expected union containing a nullable member, found nullable union
```

这是与Registry contract/identity问题独立的skiff-packages OpenAI production blocker。F375不能跨
package顺手修改。

## 7. Reverse-search evidence

| 检查 | 结果 |
| --- | --- |
| production `.skiff`中的`implements ErrorPayload` | `0` |
| Registry `api.yml`四类五项source export | `20` |
| Registry production `api.yml`中的`serviceCall: true` | `0` |
| fresh Registry `ServiceContract.operations` | `0`（blocking；预期20） |
| fresh Registry deployment/assembly ingress | `0 / 0` |
| non-test production `service.yml` | 只有`registry/service.yml` |
| 其它official package意外ServiceContract | 未发现；成功fresh发布的`http-session`和`aliyunoss`只返回PackageArtifact receipt，且其它production package均无`service.yml` |

## 8. TASK_SCOPE_EXPANDED与最小后续DAG

原任务预期“当前consumer只需重验”被四个production事实证伪，当前节点不能安全完成：

1. **skiff-packages Registry service-call authoring owner**
   - 把20个Registry callable显式标为`serviceCall: true`；
   - receipt测试必须读取真实generated ServiceContract，精确断言20个operation、20个binding、全部有
     `serviceOperationId`，不能继续只数scalar source export。
2. **skiff-packages Registry generation/storage test owner**
   - 将四类摘要schema/identity校验迁到当前`v7/v8/v6`、`v4/v4`、`v2/v2`、`v2/v2` generation；
   - 用fresh实际receipt值在隔离runtime中逐类完成immutable put/replay/read；
   - 为四类pointer各补成功CAS、read、history，并保留mismatch/limit负例。
3. **Skiff Router owner**
   - 闭合`canonicalAssemblyWebSocketIngressIdentity`的canonical定义/export/import owner；
   - 先通过Router type-check/startup probe，再恢复isolated package test。
4. **skiff-packages OpenAI owner**
   - 处理`OpenAiImageFormat?`在两个真实caller的nullable-union identity不一致，恢复完整`npm test`。

前3项中的Registry与Router production修复可以并行；合流后先跑一次fresh Registry publish +
isolated五类路径combined probe。OpenAI修复独立合流后，再由新的F375验证Agent在新的精确checkpoint上各
运行一次`npm run test:registry`和`npm test`。本worktree只有本result文档提交，没有可继续沿用的
production修改或测试PASS证据。

## 9. 自验收矩阵

| 任务条款 | 代码/record证据 | 反向搜索 | 测试 |
| --- | --- | --- | --- |
| fresh bootstrap/publish | 四对象receipt与record path见§2 | 不复用旧root | PASS |
| Registry `20 -> 20` | fresh contract `operations: []` | source 20；`serviceCall: true`为0 | **FAIL `20 -> 0`** |
| 零gateway ingress | deployment/assembly计数均0 | 只有Registry production service | PASS |
| current-generation immutable put/read | Registry校验仍为旧generation | current/frozen version表见§4 | BLOCKED；0 runtime tests |
| 四类pointer CAS/history | 只有PackageArtifact成功路径fixture | Deployment成功路径为0 | BLOCKED；0 runtime tests |
| `npm run test:registry` | 5 source tests PASS | Router missing export | FAIL |
| `npm test` | OpenAI两个caller类型不一致 | Registry尚未到达 | FAIL；0 tests |
| ErrorPayload cleanup | production `.skiff`零命中 | `rg`计数0 | source test PASS |
