# P5-F441G Official packages zero-ingress verification result

状态：`PASS / OFFLINE_ZERO_INGRESS_RECEIPT_GREEN`。official packages 的 7 个 service
root 在 strict split-manifest hard cut 后仍为 canonical zero-ingress；没有新增
`http.yml` 或 `websocket.yml`，也没有为了制造 diff 修改任何 service/config manifest。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| skiff-packages 固定输入 | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |
| skiff-packages implementation | `82c2968772927740a9ae031be11600a42c4fb099` | `44081bd0498919086c13adea97c07722cb768352` |
| Skiff production/toolchain 输入 | `67d61b8db9cb1750fe624dc40b9968642fb6d7f3` | `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff` |
| Skiff result checkout 起点 | `a33b4810aefab9b1ad60f5aaddce3b07cb53487e` | `7de8adf011739af8b912803335e9232de518716d` |

Skiff result checkout 相对 toolchain 输入只新增 F441A–F441G 调度文档；production tree
仍精确来自 `67d61b8d`。

Implementation 只修改：

- `scripts/registry-service-source.test.mjs`
- `scripts/registry-service-receipt.test.mjs`
- `scripts/test-packages.mjs`

`registry/service.yml`、`registry/config.dev.yml`、6 个 `tests/*/service.yml` 和对应
config profile 全部 bit-identical；package source/API、Skiff production、其它 task/result
均未修改。

## 2. Canonical seven-root closure

新增 source oracle 对以下 7 个 root 逐一验证：

- `registry`
- `tests/aliyunoss`
- `tests/http-session`
- `tests/openai-live`
- `tests/openai`
- `tests/registry`
- `tests/track`

每个 root 的 `package.yml`、`api.yml`、`service.yml` 都必须是 regular file；
`service.yml` 顶层 key 精确为：

- Registry：`id`、`serviceCalls`
- 6 个 test service：`id`、`kind`

6 个 test service 的 `kind` 精确为 `test`。7 个 timeout 继续由唯一对应 profile
拥有：Registry 与 5 个普通 test profile 为 `30000`，`openai-live` 为 `60000`。
每个 root 都明确拒绝存在 `http.yml` 或 `websocket.yml`。

权威反向枚举：

```bash
find registry tests \( -name http.yml -o -name websocket.yml \) -print
```

输出为空。没有伪造空 external manifest。

## 3. Registry baseline 与 current receipt

先用 hard cut 前的 clean detached Skiff toolchain `1ca6ca01` 保存 baseline receipt，
再用任务指定的 `67d61b8d` production toolchain 在同一 skiff-packages 输入上 fresh
publish/build。下表全部来自真实 producer 输出，不是手写 identity 推测。

| Receipt | Baseline | Current | 结论 |
| --- | --- | --- | --- |
| PackageBuildId | `skiff-package-build-v10:sha256:03eaa8ac4fa5d07fed60c61910bb6e658f01de70a50d0b37eb4e6774c72c9c51` | 同左 | bit-identical |
| Package Local ABI | `skiff-package-local-abi-v7:sha256:c86f9e8bc9ceee7931198af2277435950ad6b0b502b9b71d95917dc313eaafdf` | 同左 | bit-identical |
| ServiceProtocol | `skiff-service-protocol-v5:sha256:d8825672efdce323ae716e8f78152b14ec5b915f9a1eb08637be1c9b7fbc238c` | 同左 | bit-identical |
| Deployment revision | `sha256-b16c0ce2adb397e61e419c98c6b68c2803bc72f9e92d7083d8d3a91e1b91dca9` | `sha256-ab15509d70b9dae811eb9e05f9e32683f3b10504d5731a892775a71091f984e1` | schema/typed authoring hard cut 后按预期改变 |
| DeploymentArtifact | `skiff-deployment-artifact-v2:sha256:bee6bcffbb71412120c74fe5ac2568a69a9f0d583667f5bbceefc7d73f851953` | `skiff-deployment-artifact-v3:sha256:2f6eb2296a38e1c95d339dd5981bfb1fa3b69870d3a3488161fbd16e9c6aa5a3` | current v3 |
| RuntimeAssembly | `skiff-runtime-assembly-v2:sha256:905f55501a781d3ee67ec7ba56a6ceb8888bf18d3ff482929b01d748fd69c9b5` | `skiff-runtime-assembly-v2:sha256:3df8d07530e6a2cb4e267806b494be64ced10b83ffdaa006d1f8d4d84eb396f2` | new deployment ref 自然传播 |

两次 receipt 的 closure 均为：

```text
receipt operation ids       20
ServiceContract operations  20
deployment bindings         20
gateway entries              0
deployment ingress           0
assembly gateway ingress     0
```

Current oracle 现在精确要求 `skiff-service-deployment-v3` 和
`skiff-deployment-artifact-v3:sha256`，并把 `gatewayEntries`、deployment `ingress`
及 assembly `gatewayIngress` 固定为 exact `{}` / `[]` / `[]`，不再只检查长度。

任务输入的 canonical Gateway identity generation 是 F440M 冻结的
`skiff-gateway-entry-v2:sha256`。本 receipt 由同一 current toolchain 生成并消费
Deployment v3 的 gateway map；该 map 精确为空，因此不存在可诚实记录的 GatewayEntryIdentity
实例。为“证明” v2 而制造 gateway entry 或空 external manifest 都会破坏本 leaf 的
zero-ingress 目标。

## 4. Offline package-test plan

`test-packages.mjs` 新增无执行副作用的 `--list` selector。它枚举：

- 5 个 ordinary package；
- 5 个 offline test service，共 6 个 default `.test.skiff` fixture；
- `tests/openai-live` 只作为 compile-only root；
- `externalRequests: false`。

默认执行路径仍排除 `*.live.test.skiff`，没有读取外部账号或执行 live test。

在最终 implementation tree 上诊断运行默认入口时，5 个 package publish 均先成功，
Registry 仍报告 `Available: 20 / Package-only: 0`；首个 runtime test 尚未执行便在
Skiff 的既知 R0 follower 边界停止：

- `runtime/eval/src/runtime_http_gateway.rs:85` 尚未消费
  `GatewayAdapterKind::WebSocketJsonRpc`；
- 同文件 `:439` 与
  `runtime/eval/src/runtime_websocket_connect.rs:171` 尚未消费
  `WebSocketJsonRpcParams` / `WebSocketBusinessIdentity`。

三个错误都是 exhaustive-match `E0004`，与 F440M result 记录的 Router/Runtime
后继 blocker 相同。它们不属于本 leaf 写集；没有用 wildcard、compatibility alias
或 Skiff production 越界修复。任务允许包含 external/live fixture 的 package runner
使用 list/dry-run selector，因此本 leaf 采用 `--list` 作为非 live gate。该诊断没有进入
package test body、外部请求、stable 或 live target。

## 5. 最终验证

| 命令 | 结果 |
| --- | --- |
| `SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f441g-official-packages-result node --test scripts/registry-service-source.test.mjs scripts/registry-service-receipt.test.mjs` | PASS，`8 passed / 0 failed / 0 skipped` |
| `node scripts/test-packages.mjs --list` | PASS，枚举 6 个 test service root；无执行、无 external request |
| `node --check scripts/test-packages.mjs` | PASS |
| `npm run type-check` | PASS |
| `find registry tests \( -name http.yml -o -name websocket.yml \) -print` | PASS，空输出 |
| `git diff --check` | PASS |
| implementation changed-file boundary | PASS，仅第 1 节 3 个 script |

未访问外部服务、stable instance、fixed port 或 live target；未 merge、rebase、push，
未派子 agent。Implementation 提交后 clean；本文为独立 result-only 提交。
