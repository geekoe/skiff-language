# P5-F445H-I7-P1X Timeout domain independent acceptance result

状态：

```text
PASS
P1X_COMPLETE = YES
P1_ACCEPTED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

## 1. 独立输入与范围

| 项 | 冻结值 |
| --- | --- |
| Skiff candidate commit/tree | `07090bc3b13025f4dfc24f6413bdf225010c56b1` / `772384d25095fe524fcac331839a601111a23ac3` |
| 初始Internals commit/tree | `fb0030be1175c1cc29c572401bcd921aa9676ee3` / `3b42bd3a84aaf4862b414efdb2c8421fe4392adf` |
| M迁移后的Internals commit/tree | `7fa2ac5de5a576013ee2be74032435a361c8a6e4` / `dcf91f0243e230ea5eff03f1f00ac2d7990d325b` |
| official packages commit/tree | `b06d7aaf16b6914837de1f74920fd3f626040472` / `fb9db28a7d1bd3babafd1dfa7a23687e393ff856` |

预检在零worktree状态完成；三个输入仓库均为clean。验收只读production和tests，写集只有本result。
没有修改production、test或Internals，也没有push。

## 2. Timeout domain复验

三个域保持独立：

| 域 | 唯一预算与复验结果 |
| --- | --- |
| external business request | `requestTimeoutMs`；只进入HTTP/WebSocket业务dispatch |
| assembly activation prepare | `activation.prepareTimeoutMs`；缺省`120000ms` |
| WebSocket generation release | lifecycle私有缺省`5000ms` |

Router config完整suite覆盖并通过：

- 未声明`activation`且`requestTimeoutMs = 7000`时，prepare仍缺省为`120000`；
- YAML与CLI显式值可独立设为`120000`、`150000`；
- `0`、负数、小数、字符串、对象和超safe-integer全部fail closed；
- coordinator假时钟在`20001ms`和`119999ms`仍保持pending，到`120000ms`才产生
  `assembly activation prepare timed out`并abort；
- `AssemblyActivationCoordinatorOptions`只有`prepareTimeoutMs`，不接收`requestTimeoutMs`；
- control-plane只把coordinator的prepare timeout分类为HTTP `504`，业务request预算不能生成activation
  timeout；
- WebSocket generation release在独立`5000ms`到期，未继承business或activation预算。

Test-runner完整suite同时固定：

```text
ACTIVATION_HTTP_TIMEOUT = 150000ms
BUSINESS_HTTP_TIMEOUT = 30000ms
150000ms > 120000ms
```

两个call site分别只消费自己的deadline；没有退回共享`HTTP_TIMEOUT`。

## 3. 静态与完整测试证据

| 命令 | 结果 |
| --- | --- |
| `npm run type-check`（`router/`） | PASS |
| `npm test`（`router/`） | `59 files / 842 tests passed` |
| `cargo test --locked --manifest-path test-runner/Cargo.toml` | `73 passed / 3 existing ignored / 0 failed` |
| runtime-stack config/deploy/instance Node tests | `25 passed / 0 failed` |

Rust输出只有继承的unused/dead-code warning，不影响验收结论。

## 4. Hermetic isolated动态证据

在初始Internals输入上运行current helper：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/codex-relay
```

Runtime日志给出：

```text
14:15:57.442943 runtime.assembly_candidate_started
14:16:26.164020 runtime.assembly_prepared
14:16:26.173134 runtime.assembly_committed
14:16:35.261616 runtime.assembly_request_error
```

`candidate_started -> prepared = 28.721077s`，已超过旧的`20s requestTimeoutMs`，但仍正常
prepared并committed。随后test-runner进入真实测试请求并报告`1 test(s) failed`，因此不是zero assertion，
也没有activation HTTP `504`。后续失败是：

```text
request-local exception does not support ordinary member access
```

它属于已知Skiff compiler/runtime语义阻塞，不属于P1 timeout domain。整次helper含冷编译和清理耗时
`166.35s`；P1关键的prepare阶段耗时以上述Runtime时间戳为准。

## 5. M迁移阻塞的明确分类

按M迁移后的Internals输入重跑AIHub helper，`22.54s`内在activation之前失败：

```text
service dependency agine.ai/codex-relay@0.1.0 has no published ServiceContract pointer
```

M helper把带service依赖的AIHub subject package放入packages阶段先publish，而Relay ServiceContract仍在
后续services阶段才publish。该错误没有出现`runtime.assembly_candidate_started`，是M test-only
publish排序/closure阻塞，不是P1回归。本验收没有越权修改M，也不声称Relay、AIHub、M或I7整体完成。

Agine同样保留给M结构和既有compiler阻塞的后续owner；不把`includeTarget`当作P1修复。

## 6. 结论

Router config、coordinator裁决、control-plane分类、WebSocket release和test-runner client预算均通过独立
静态与完整suite复验。更重要的是，真实Relay activation prepare持续`28.721077s`后仍prepared/committed，
直接否定旧`20s`业务请求预算仍控制activation的可能。

```text
P1X_COMPLETE = YES
P1_ACCEPTED = YES
RELAY_COMPLETE = NO
AIHUB_COMPLETE = NO
M_COMPLETE = NO
I7_COMPLETE = NO
```

最终result commit/tree由Git handoff记录；本文不自引用自身commit。
