# P5-F413 Codex Relay serviceCalls and HTTP checkpoint migration result

状态：Complete（Relay-owned 迁移已合流；真实 authoring 暴露 suspension 后继 blocker）。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| Internals start | `3a7234610c53b11c5f2cfdb5b04448408e924e31` | `72901006d82f6abafef3efb6567e4ccba6aa4caf` |
| F367 checkpoint | `68c7d679899bf942060fe407270cea60b7ba85ca` | `de19e938259e6f023cd206791f4bfb9c5e4d03d9` |
| agent implementation | `950ff92db9c4da2e0205c10a7659fefc79b06ba0` | `f36a387cbe9740050d755c6fe247c3b509a68767` |
| integration cherry-pick | `960cc4bd722cbbad41fdd5e064663ad505e4f3ac` | `33a838176990193cd01be495a7b692623baa4793` |
| final validation Skiff | `e8d5c12f208c5b6b6f68d9a98ac47a36a9839056` | `ac99f26d67cf71a1d04c292b3e02ea041d8140fa` |

改动只落在：

```text
codex-relay/service/api.yml
codex-relay/service/relay.skiff
codex-relay/service/service.yml
codex-relay/service/service-api-receipt.test.mjs
```

没有带入 F367 分支祖先的 Account 改动。

## 2. Checkpoint 移植与新模型适配

- `service.yml` 精确移植 F367 的 30 个 named `rawHttp` entries；
- 删除 legacy `routes`、`operation` 与 service-level duplicate timeout；
- `serviceCalls` 只选择 `relayProxy`；
- 30 个 HTTP entries 中 27 个为 unary、3 个 `/v1` entries 为 server stream；
- 每个 entry 精确有一个 `request <- http.request` adapter source；
- `api.yml` 删除 15 个 external-only scalar exports，保留两个类型与完整
  `relayProxy const + interfaces` block；
- 没有移植旧 `serviceCall: true` marker；
- `relay.skiff` 只移植两个 interface method 的 `self: Self` receiver hunk；
- 除新增 `serviceCalls`、删除旧 marker 外，三份 production 文件与 F367 checkpoint
  各自 patch-equivalent。

目标 ServiceContract 精确为：

```text
relayProxy.responsesCompleted
relayProxy.responsesCompletedResult
```

HTTP gateway 不进入 service operation 集合。receipt oracle 已切为 protocol v4、operation ID v1，
并验证 2 operations、30 gateways/ingress、handler link 与 selector 闭合。

## 3. 聚焦验证

- Relay static Node：`4 passed / 0 failed`；
- Node syntax check：PASS；
- 静态计数：`30 rawHttp / 30 handlers / 30 http.request sources`；
- production legacy 搜索：0；
- `git diff --check`：PASS。

## 4. 真实 authoring 暴露的后继 blocker

isolated owner 已越过旧 Relay manifest parse，并在 Package identity projection 精确失败：

```text
contract validation failed:
package agine.ai/codex-relay@0.1.0 identity projection failed:
PackageArtifact is invalid:
public instance relayProxy method responsesCompleted return or suspension semantics disagree with its interface
```

当前 interface method descriptor 固定为 `may_suspend=false`，实现经现有推断为 suspending；这是已排队的
suspension semantic owner，不属于 Relay authoring。P5-F413 没有通过改业务实现、放宽验证或改共享
compiler 掩盖该失败。该真实用例进入 suspension 后继任务验收；后继完成后必须重跑 Relay 与 AIHub 的
isolated authoring/type-check。
