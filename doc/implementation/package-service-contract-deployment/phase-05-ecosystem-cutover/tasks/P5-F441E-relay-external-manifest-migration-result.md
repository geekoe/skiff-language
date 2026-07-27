# P5-F441E codex-relay external manifest migration result

状态：`PASS / RELAY_LEAF_GREEN / SHARED_TYPECHECK_EXPECTED_BLOCKER`。Relay 的 30 个 HTTP
entry 已从 `service.yml` 一次性 hard cut 到顶层 direct-map `http.yml`；PackageBuildId 与
ServiceProtocolIdentity 改前改后逐字相同，新的 deployment 为 v3、全部 gateway identity 为 v2。
没有修改 Relay handler、上游 HTTP transport 或 raw stream 实现。

## 1. 输入、提交与唯一写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| Internals 固定输入 | `8ccc6cc5a066e674964c3b88e86316d67adfcb1a` | `817591e145395bc514538a0480decc4e5be9f1f0` |
| Skiff 指定 toolchain | `67d61b8db9cb1750fe624dc40b9968642fb6d7f3` | `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff` |
| Skiff result worktree 起点 | `a33b4810aefab9b1ad60f5aaddce3b07cb53487e` | `7de8adf011739af8b912803335e9232de518716d` |
| Internals implementation | `34fcbfa9cb00e99d140ac056dec16b92dc6edf60` | `d5bcc1594259b2d5b4cc1244900fb82a66cf252f` |

result worktree 起点是指定 toolchain 的直接后继，只增加 phase-05 调度文档。执行前用

```bash
git diff --quiet 67d61b8db9cb1750fe624dc40b9968642fb6d7f3 HEAD -- . ':(exclude)doc/**'
```

证明 compiler/toolchain production 内容相同。

Implementation 精确修改：

- 新增 `codex-relay/service/http.yml`；
- 精简 `codex-relay/service/service.yml`；
- 修改 `codex-relay/service/service-api-receipt.test.mjs`。

`config.dev.yml` 在授权写集内但无需改字节；其 SHA-256 改前改后均为
`471a8f64d7d2b09b49cd873339372c34638cabdba8b68617930483d01c5ad918`。
没有修改第四个 service、shared workflow、Skiff production 或历史 task/result。

## 2. 迁移前 baseline receipt

修改 source 前先运行 direct receipt：

```bash
node --test codex-relay/service/service-api-receipt.test.mjs
node --check codex-relay/service/service-api-receipt.test.mjs
```

结果为 `4 passed / 0 failed` 和 syntax PASS；它固定了两个 service operation、30 个 named
`rawHttp` entry 及 `27 unary / 3 server-stream`。旧 `service.yml` SHA-256 为
`94493c9496074192100a52ddadc988993d002376b289c6a098fa7a6f27c8c987`。

因为指定的新 strict reader 会按设计拒绝旧 inline `service.yml.http`，baseline typed receipt 使用
F440H 固定的 pre-hard-cut compiler 输入
`a829bde6d250cd348a28f25c6246de6cbed2df9e`，只在 fresh 临时 artifact/cargo root 中发布
std、`llm-api`、`llm-providers` 与 Relay。F440H/F440M 已明确 PackageArtifact 和
ServiceContract generation 未变；该旧 compiler 只负责合法读取迁移前 authoring。

保存的旧 receipt：

```text
/private/tmp/p5-f441e-relay-baseline.sG1lT8/receipts/codex-relay.json
```

| baseline fact | exact value |
| --- | --- |
| PackageBuildId | `skiff-package-build-v10:sha256:20372f1d62cfd5527371924f6ecc9c7967decb4950900942b0fb66c10ce44303` |
| ServiceProtocolIdentity | `skiff-service-protocol-v5:sha256:c1e3f8be3d63b3b2864eb7f36d5f92dad4014005915a5a1bda3590e1ea6649fa` |
| DeploymentArtifactIdentity | `skiff-deployment-artifact-v2:sha256:00a05d60417bc372f296c3d968621791d42f1eacf263f7c06c729fe9e4a775a8` |
| Gateway / ingress | `30 / 30` |
| Dispatch | `27 unary / 3 serverStream` |
| Gateway generation | 全部 `skiff-gateway-entry-v1` |

全过程没有读取 stable artifact root、watch registry、instance 或 live target。

## 3. External manifest hard cut

`service.yml` 现在精确为：

```yaml
id: agine.ai/codex-relay
serviceCalls:
  - relayProxy
```

旧 inline `http:` 下的 30 个 entry 仅去掉 wrapper 和两格缩进，逐项原样成为
`http.yml` 顶层 mapping。以下机械比较为零 diff：

```bash
diff -u \
  <(git show 34fcbfa^:codex-relay/service/service.yml |
    sed -n '/^http:/,$p' | tail -n +2 | sed 's/^  //') \
  codex-relay/service/http.yml
```

因此 key、method、path、`rawHttp` kind、handler、`request <- http.request` adapter 均未改变。
`config.dev.yml` 继续是 timeout 的唯一 owner，生成 deployment 的 `timeoutMs` 精确为 `120000`。

Receipt parser 现在从 `http.yml` 第一行开始读取 direct map，不再寻找 `service.yml` 中的
`http:` index，也没有 wrapper/slice compatibility。它同时：

- exact 固定精简后的 `service.yml`；
- 拒绝 `service.yml` 的 `http/websocket/timeout` owner；
- 拒绝 `http.yml` 的 `http/routes/entries` wrapper；
- 把 synthetic/real record oracle 收敛到 Gateway v2、DeploymentArtifact v3；
- 使用 compiler 的 canonical omitted-host projection `host: "*"`。旧 baseline 和新 record 都是
  `*`，因此这只是修复旧 synthetic 空串，不是 selector 变化。

## 4. 新 typed record 与 identity 比较

使用指定 Skiff toolchain production 内容，在另一个 fresh 临时 artifact/cargo root 中发布同一依赖闭包
和迁移后的 Relay。保存的新 receipt：

```text
/private/tmp/p5-f441e-relay-current.WbuGpd/receipts/codex-relay.json
```

新 record：

| fact | exact value |
| --- | --- |
| PackageBuildId | `skiff-package-build-v10:sha256:20372f1d62cfd5527371924f6ecc9c7967decb4950900942b0fb66c10ce44303` |
| ServiceProtocolIdentity | `skiff-service-protocol-v5:sha256:c1e3f8be3d63b3b2864eb7f36d5f92dad4014005915a5a1bda3590e1ea6649fa` |
| DeploymentArtifactIdentity | `skiff-deployment-artifact-v3:sha256:2904027640a5ff18cfced9004b3cd6ce92e33d9f0d9a4c751895c87b2adab298` |
| Gateway / ingress | `30 / 30` |
| Dispatch | `27 unary / 3 serverStream` |
| Gateway generation | 全部 `skiff-gateway-entry-v2` |
| timeoutMs | `120000` |

比较脚本对 baseline/current 做了以下 exact assertions，全部 PASS：

1. PackageBuildId 逐字相同；
2. ServiceProtocolIdentity 逐字相同；
3. 去掉按 generation 必然变化的 `gatewayEntryIdentity` 后，30 个 entry 的
   handler、adapter plan 和完整 protocol surface 逐字相同；
4. 30 个 ingress selector 逐字相同；
5. 所有 adapter kind 都是 `rawHttp`；
6. server-stream key set 精确为
   `v1ModelsGet`、`v1ResponsesCompactPost`、`v1ResponsesPost`。

新的 full PackageArtifact、ServiceContract、ServiceDeployment record 又实际传入
`assertCodexRelayServiceApiReceipt` 与 `assertCodexRelayGeneratedRecords`，结果 PASS。

## 5. Raw HTTP stream 语义

本 leaf 没有修改任何 `.skiff` source。尤其没有修改：

- `proxy_runtime.proxy` handler；
- `std.http.stream` 上游请求；
- `Stream<std.http.HttpResponseStreamEvent>` 返回；
- status/header/body chunk 的 raw 转发；
- SSE buffering、response byte limit 或 interaction 记录。

Manifest 级三个 `proxy_runtime.proxy` binding 与 baseline 完全相同；compiler 生成的三条
`dispatchMode: serverStream` protocol surface 也逐字相同。其余 27 条仍为 raw HTTP unary。
因此本迁移没有把 raw stream 投影成 typed JSON chunk，也没有改变 handler 或传输格式。

## 6. 验证与 shared 遮挡

| 命令 / probe | 结果 |
| --- | --- |
| `node --test codex-relay/service/service-api-receipt.test.mjs` | PASS，`4/4` |
| `node --check codex-relay/service/service-api-receipt.test.mjs` | PASS |
| fresh isolated Relay package publish | PASS，Package/Contract/Deployment 三类 typed receipt |
| real generated-record receipt validator | PASS |
| baseline/current identity + raw gateway comparator | PASS |
| `git diff --check` | PASS |
| `npm run type-check`（`codex-relay/service`） | 命令已执行；exit 254，目录没有 `package.json`，在任何 check 前 ENOENT |
| direct canonical workflow owner | Relay publish PASS；随后在 AIHub legacy manifest 的首错停止 |

Relay 没有 npm package/script，且新增 wrapper 不在唯一写集，因此未伪造 `npm run type-check` pass。
随后运行等价的 canonical owner：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f441e-relay-manifest-result \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
CARGO_TARGET_DIR=/private/tmp/p5-f441e-relay-current.WbuGpd/cargo-target \
node scripts/check-isolated-service-graph.mjs agine.ai/codex-relay
```

workflow 先成功发布 Relay，下一 service 精确首错为：

```text
failed to parse .../aihub/service/service.yml:
unknown field `http`, expected one of `id`, `kind`, `serviceCalls`
at line 7 column 1
```

这是任务预先允许记录的跨 service external-manifest 遮挡；本 leaf 没有修改 AIHub。

## 7. Reverse search 与隔离

- `service.yml` 不再匹配任何顶层或内联 `http/websocket/timeout`；
- `http.yml` 精确有 `30` 个顶层 key、`30 rawHttp`、`30 http.request` source 和
  `3 proxy_runtime.proxy` handler；
- implementation commit 只列第 1 节三个文件，`config.dev.yml` byte-identical；
- implementation worktree 提交后 clean；
- 未启动 instance、watch、reload、Mongo、Router、Runtime 或固定端口 workload；
- 未访问 stable/live，未 merge、rebase、push，未派子 agent。

本文是独立 result-only 提交；result commit/tree 与最终 clean 状态由交付消息记录。
