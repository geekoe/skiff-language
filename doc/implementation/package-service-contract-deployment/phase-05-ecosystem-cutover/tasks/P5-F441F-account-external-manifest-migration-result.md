# P5-F441F Account external manifest migration result

状态：`PASS / ACCOUNT_LEAF_GREEN`。Account 的 21 个 raw HTTP entry 已从 inline
`service.yml.http` hard cut 到顶层 direct-map `http.yml`；PackageArtifact 与零操作
ServiceContract 的完整 record bytes、PackageBuildId 和 ServiceProtocolIdentity 均与迁移前 exact
相等。共享 Internals type-check 按任务预期被尚未迁移的 Relay manifest 遮挡，没有越界修改。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 固定 Internals 输入 | `8ccc6cc5a066e674964c3b88e86316d67adfcb1a` | `817591e145395bc514538a0480decc4e5be9f1f0` |
| 固定 Skiff toolchain 输入 | `67d61b8db9cb1750fe624dc40b9968642fb6d7f3` | `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff` |
| Skiff leaf dispatch HEAD | `a33b4810aefab9b1ad60f5aaddce3b07cb53487e` | `7de8adf011739af8b912803335e9232de518716d` |
| Internals implementation | `a0445670713682ab0c52b24e7ddb2afdffaf052d` | `b5b32a75ed552d61e71ef9d640a1db1a27c23d20` |

`a33b4810` 相对指定 toolchain 输入只新增七份 F441 调度 task；production/toolchain 文件没有变化。

Implementation 只修改：

- `skiff-platform/account/service.yml`
- `skiff-platform/account/http.yml`
- `skiff-platform/account/service-api-receipt.mjs`
- `skiff-platform/account/service-api-receipt.test.mjs`

`config.dev.yml` 已经是唯一正确 timeout owner，内容与输入 commit exact 相等，因此没有制造无语义 diff。
Skiff 侧只新增本文。

## 2. Baseline receipt

修改 Account 前先运行原 direct receipt，结果 `6 passed / 0 failed`。随后在临时 artifact store 中，
用 hard-cut 前的本地 compiler checkpoint
`1ca6ca01b28fecbef2ea599700f54a2d3020632e` 和固定 Internals 输入，仅构建 Account 的精确依赖闭包
`std + http-session + Account`。baseline source/toolchain worktree 在 receipt 保存后均已删除。

Baseline canonical receipt：

| 域 | 值 |
| --- | --- |
| PackageBuildId | `skiff-package-build-v10:sha256:b5058aff6df1cb9b91b92583d63413676caf4c1d57bdc6ec6a86051fd60197a9` |
| PackageLocalAbiIdentity | `skiff-package-local-abi-v7:sha256:969316ccffb53363675920a2d8442d69f1a6c6c99cc202e7d816290664efd246` |
| PackageArtifact record bytes | `380257` bytes；SHA-256 `c6d84e4989287c62d6684150d9ee3fe0937497e62b52da3421cc8fad911e27ce` |
| ServiceProtocolIdentity | `skiff-service-protocol-v5:sha256:62e6eb806c368ec041b3a8d74318d3fb068fd50356142d9326668490efd1e7cf` |
| ServiceContract record bytes | `350` bytes；SHA-256 `f7db016ba5542c96554d6bb02898afd194586623002d57e4d46a5c88f0bd62f1` |
| Contract closure | `0` operations；`0` package type requirements；22 package-visible functions均无`serviceOperationId` |
| Old deployment | `skiff-service-deployment-v2`；artifact `skiff-deployment-artifact-v2:sha256:d847cadc0ab2e322c9c17c9a55c38414df0db3a613f7e94c331d94444d8a0d2b` |
| Old deployment closure | 21 Gateway v1 entries；21 ingress；0 operation bindings；`timeoutMs=120000` |

## 3. Migration

- `service.yml` 现在 exact 为一行 `id: skiff.run/account`。
- 原 inline map 的 21 个 entry 逐行去掉共同的两空格缩进后写入顶层 `http.yml`；entry key、顺序、
  `POST` selector、path、`rawHttp` kind、`account.<key>` handler 和
  `request <- http.request` adapter 均保持。
- `service-api-receipt.test.mjs` 直接读取 `http.yml`，不再读取 `service.yml` 后依赖
  `lines[1] == "http:"` 或 slice 旧 inline wrapper。
- receipt 消费当前 generation：ServiceProtocol v5、PackageBuild v10、Gateway v2、
  ServiceDeployment v3、DeploymentArtifact v3。
- receipt 继续固定 zero-operation Contract、21-entry gateway/ingress exact closure、unary raw HTTP
  surface和`timeoutMs=120000`。

Test-first red 在只更新新契约测试后真实得到 `1 passed / 5 failed`：旧 service 仍含 inline HTTP、
`http.yml` 尚不存在，且旧 receipt 仍只接受 stale generation。生产迁移和 receipt 更新后全部转绿。

## 4. 迁移后 canonical receipt 与 exact 边界

使用指定 Skiff lineage、固定 skiff-packages
`f8c634ce4573506e35f6bc1c7cc1e4eef9992a78`，在临时 artifact store 重建 Account，并直接把生成的
PackageArtifact、ServiceContract、ServiceDeployment 交给本 leaf 的 receipt validator。

| 域 | Baseline → migrated | 结论 |
| --- | --- | --- |
| PackageArtifact record bytes | `380257 / c6d84e49...27ce` → 同值 | exact 相等 |
| PackageBuildId | `b5058aff...197a9` → 同值 | exact 相等 |
| PackageLocalAbiIdentity | `969316cc...fd246` → 同值 | exact 相等 |
| ServiceContract record bytes | `350 / f7db016b...62f1` → 同值 | exact 相等 |
| ServiceProtocolIdentity | `62e6eb80...1e7cf` → 同值 | exact 相等 |
| Contract operations / requirements | `0 / 0` → `0 / 0` | exact zero-operation closure |
| Deployment revision | `sha256-9d2c51af...8fc2e` → `sha256-7eee303f...ce6b5` | 按 external authoring hard cut 改变 |
| Deployment artifact | v2 `d847cadc...a0d2b` → v3 `17add456...f66f2` | current generation |
| Gateway / ingress | 21 v1 / 21 → 21 v2 / 21 | selector/handler/adapter closure保持 |
| timeout | `120000` → `120000` | 仅来自既有 profile |

新 deployment 的 exact identity 为
`skiff-deployment-artifact-v3:sha256:17add45609598b509edee2e0d7d4f10bc841e15030207bbb5c32cf7aa8cf66f2`；
schema为`skiff-service-deployment-v3`，operation bindings为0。

## 5. 验证

| 命令 / 探针 | 结果 |
| --- | --- |
| `node --test skiff-platform/account/service-api-receipt.test.mjs` | PASS，6 passed |
| `node --check skiff-platform/account/service-api-receipt.mjs` | PASS |
| `node --check skiff-platform/account/service-api-receipt.test.mjs` | PASS |
| Account-only canonical build + real generated-record validators | PASS |
| baseline / migrated full PackageArtifact与ServiceContract byte digest对比 | PASS，两个record均 exact |
| 旧 inline map机械去缩进后与新`http.yml`逐字节比较 | PASS |
| `git diff --check` | PASS |
| `npm run type-check`（`aihub/service`共享 canonical workflow） | 预期跨 service 遮挡；见下文 |

共享 type-check 使用：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f441f-account-manifest-result \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
npm run type-check
```

它在 Account 之外的首个旧 manifest 处停止：

```text
codex-relay/service/service.yml:4:1
unknown field `http`, expected one of `id`, `kind`, `serviceCalls`
```

这是任务明确允许记录的 Relay leaf 遮挡；Account-only canonical build已经使用同一指定 toolchain和隔离
artifact store独立通过。未修改 Relay 或 shared workflow。compiler 输出的 unused/dead-code warning为既有
warning，不影响 receipt。

## 6. Reverse search 与隔离

- `service.yml` exact 只有 service id；其中无`http/websocket/timeout`。
- `http.yml` 有21个唯一 key和21个唯一`method + path` selector，无`routes`、`operation`或wrapper
  `http:`。
- Account receipt 文件中已无`lines[1]` inline-wrapper依赖、Gateway v1、DeploymentArtifact v2或
  ServiceProtocol v4。
- `config.dev.yml`与输入 bytes相同，仍精确拥有`timeout: 120000`。
- implementation commit 后 Internals clean；本文提交后的 Skiff clean状态由交付消息记录。
- 只使用临时隔离 artifact store；未访问 instance、watch、reload、固定端口、stable或live。
- 未派子 agent，未 merge、rebase或push。
