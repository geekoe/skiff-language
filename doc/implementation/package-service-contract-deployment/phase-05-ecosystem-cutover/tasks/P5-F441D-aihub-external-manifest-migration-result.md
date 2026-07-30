# P5-F441D AIHub external manifest migration result

状态：`PASS / AIHUB_SPLIT_MANIFEST_GREEN`。AIHub 已 hard cut 到独立 `http.yml`；PackageBuildId 与
ServiceProtocolIdentity 相对迁移前 canonical record exact 相等，新 deployment 为 v3、7 个 gateway /
ingress，全部 GatewayEntryIdentity 为 v2。共享 Internals type-check 被未迁移的 Codex Relay manifest
按任务预期遮挡，AIHub direct receipt 已独立闭合。

## 1. 输入、基线与提交

| 项目 | Commit | Tree |
| --- | --- | --- |
| Internals 固定输入 | `8ccc6cc5a066e674964c3b88e86316d67adfcb1a` | `817591e145395bc514538a0480decc4e5be9f1f0` |
| Skiff production toolchain 输入 | `67d61b8db9cb1750fe624dc40b9968642fb6d7f3` | `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff` |
| Skiff result worktree dispatch HEAD | `a33b4810aefab9b1ad60f5aaddce3b07cb53487e` | `7de8adf011739af8b912803335e9232de518716d` |
| read-only skiff-packages 输入 | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |
| Internals implementation | `73d7b67c4e23fa55d4d6b3bf8922497d011c2640` | `e01faf3d06d8b082c4a8026961047b80f5d1de50` |

result worktree dispatch HEAD 相对指定 toolchain 输入只新增 F441A–F441G task 文档；排除该 task 目录后
`git diff 67d61b8d..a33b4810` 为零，因此构建实际使用的 production toolchain 与任务冻结输入 exact。

在修改 Internals authoring 前，先引用
`P5-F437A-aihub-canonical-publish-path-closure-audit-result.md` 的既有 isolated canonical
AIHub record：

- PackageBuildId：
  `skiff-package-build-v10:sha256:17624dcc945815d35a320cdea10bf5b5859c91db6799cfbf473a66d035b60ed8`
- ServiceProtocolIdentity：
  `skiff-service-protocol-v5:sha256:d8ef2bc6315089561746a3922c570663382cd6609302e25cf4f2a4ec9d54b4e7`

该 record 的 Internals 输入是
`066b5135a8e06f87acfd614e408e05b35453f4eb`。迁移固定输入的 `aihub/service` subtree 与它
bit-identical，二者 tree object 都是
`54e9320feb9fcb9b8a69d6ce6be8dbefc6c0b396`。F440H/F440M 同时冻结了 external manifest split
不改变 PackageArtifact / ServiceContract generation，因此这是本 leaf 的有效迁移前 baseline。

## 2. 实现

implementation commit 精确修改四个文件：

- `aihub/service/service.yml`
  - 只保留 `id: agine.ai/aihub`；
  - 保留且只保留既有两个 `serviceCalls`：`managedLlm`、`providerCatalog`；
  - 删除 inline `http` 与重复 `timeout` owner。
- 新增 `aihub/service/http.yml`
  - 顶层为 7-entry direct map；
  - keys、method、path、kind、handler 和 adapterArgs 相对旧 inline body 逐字节等价，仅统一去掉旧
    `http:` wrapper 所需的两格缩进；
  - normalized bytes SHA-256 为
    `8be85304e2dbe01a20f31dcfdfc315cc8f6efa6e0a4792274b427a9c0da7ed34`。
- `service-api-receipt.mjs` / `service-api-receipt.test.mjs`
  - receipt oracle 直接读取 `http.yml`，不再从 `service.yml` 截取 `http` 字符串；
  - 固定 7 个 selector、handler、raw request adapter及 5 unary / 2 server-stream；
  - 新增 full generated-record validator，要求 Package/Contract closure、deployment v3、
    DeploymentArtifact v3、GatewayEntry v2、7 ingress和 timeout 120000；
  - stale deployment v2、gateway v1、缺 ingress和错误 stream dispatch均有 terminal negative。

获授权的 `aihub/service/config.dev.yml` 经核对内容 exact 不变，继续唯一拥有 scalar
`timeout: 120000`，因此不制造无内容 diff。

旧 inline body 与新 `http.yml` 的 exact comparison 得到：

```text
entries = 7
keys = healthGet, v1ProvidersGet, v1ModelsGet, v1ChatEventsPost,
       chatEventsPost, v1ChatCompletionsPost, chatCompletionsPost
normalized bytes equal = true
```

没有修改 AIHub source/API/profile、Agine、Relay、Account、shared workflow、Skiff production或其它
task/result。

## 3. Direct canonical receipt

direct receipt 使用唯一临时 root `/tmp/p5-f441d-aihub.47v4YP`，`CARGO_TARGET_DIR`、artifact store、
logs和生成记录都位于该 root。依赖闭包按下列顺序写入 fresh temporary store：

1. 指定 Skiff toolchain bootstrap `skiff.run/std`；
2. 当前 Internals `llm-api`、`llm-providers`；
3. 当前 Codex Relay root 的只读临时副本；只在临时副本把 inline HTTP 拆成 strict
   `service.yml + http.yml`，没有写 Relay worktree；
4. 本 implementation worktree 的真实 `aihub/service` root。

临时 Relay副本只用于提供 AIHub 所需的 exact dependency Package/Contract records；未进入提交、
未被当作 Relay migration verdict，也未访问 stable store。AIHub receipt 的三份 raw record随后由本
leaf 的 `assertAihubServiceApiReceipt` 与 `assertAihubGeneratedRecords` 直接读取并通过。

| Record / invariant | 迁移后 exact 结果 | 与 baseline |
| --- | --- | --- |
| PackageArtifact | schema v9；build `skiff-package-build-v10:sha256:17624dcc945815d35a320cdea10bf5b5859c91db6799cfbf473a66d035b60ed8` | exact equal |
| PackageLocalAbi | `skiff-package-local-abi-v7:sha256:3f825cc43bd9a6fe7942e260329c2b10d28d62a717b2c947a9c4c8916783ca35` | external split不改 owner |
| ServiceContract | schema v5；5 operations；protocol `skiff-service-protocol-v5:sha256:d8ef2bc6315089561746a3922c570663382cd6609302e25cf4f2a4ec9d54b4e7` | exact equal |
| ServiceDeployment | schema `skiff-service-deployment-v3`；revision `sha256-a4a185b63f3d5870b3c9329ab400539d305ca2feed55260e56aeffde3b20fe85` | 新 external owner |
| DeploymentArtifact | `skiff-deployment-artifact-v3:sha256:7761d09d8b039ae6e47cd8c39c2bccd3514b343d8134d67da7691851fbc05680` | current v3 |
| Gateway / ingress | `7 / 7`；all `skiff-gateway-entry-v2`；5 unary / 2 server-stream | PASS |
| policy | `timeoutMs = 120000` | 只来自 `config.dev.yml` |

五个 unary entry都精确绑定 `internal.aihub_service.handleAihubHttp`；两个 server-stream entry都精确
绑定 `internal.aihub_service.handleAihubEventsHttp`。所有 selector 保留 host `*` 及原 method/path，
adapter plan均为 `rawHttp` + `request <- http.request`。

## 4. 共享 type-check 遮挡

任务规定的 `npm run type-check` 使用显式：

```text
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f441d-aihub-manifest-result
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration
```

并将 TMPDIR、Cargo target与 npm cache指向上述临时 root。它完成 toolchain build与前置 package stage后，
在第一个 service root Codex Relay 按 strict DTO fail closed：

```text
failed to parse service source control file
/Users/geek/workspace/internals-p5-f441d-aihub-manifest/codex-relay/service/service.yml:
unknown field `http`, expected one of `id`, `kind`, `serviceCalls`
at line 4 column 1
```

这是任务预声明的“其它 service 旧 manifest 遮挡”；AIHub 尚未由共享 workflow 执行。本 leaf 没有修改
Relay，而是按第3节完成 AIHub direct receipt。

## 5. 验证

| 命令 / probe | 结果 |
| --- | --- |
| `node --test aihub/service/service-api-receipt.test.mjs` | PASS：10 discovered，8 pass / 0 fail / 2 generated-owner skip |
| 同一 test 注入真实 `SKIFF_SERVICE_API_RECEIPT` 与 `SKIFF_SERVICE_BUILD_RECORDS` | PASS：10 / 10，0 skip |
| `node --check aihub/service/service-api-receipt.mjs` | PASS |
| `node --check aihub/service/service-api-receipt.test.mjs` | PASS |
| `npm run type-check` | EXPECTED BLOCKED：第4节 exact Relay 首错 |
| 指定 toolchain AIHub direct publish + raw-record validator | PASS |
| 旧 inline HTTP normalized bytes vs 新 `http.yml` | PASS，7 entries exact equal |
| `git diff --check` | PASS |

反向搜索结果：

- `service.yml` 中顶层 `http|websocket|timeout` 与 legacy `routes|operation`：0；
- `http.yml` 中 wrapper `http|entries|routes` 与 legacy `operation`：0；
- receipt source中没有从 `service.yml` slice HTTP body的兼容 parser；
- implementation commit只含第2节四个文件，共 `381 insertions / 94 deletions`。

未运行或访问 stable watch/reload、stable artifacts、router/runtime instance、固定端口、MongoDB、
provider或任何 live workload。未 merge、rebase、push，未派子 agent。Internals implementation 提交后
clean。可重建临时 root 已移入 macOS Trash，并验证原 path absent；清空 Trash 前仍可恢复。
result-only commit及其最终 clean状态由交付消息记录。
