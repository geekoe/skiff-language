# P5-F445H-I7-A Agine service, protocol and Host terminal result

状态：

```text
BLOCKED
A_COMPLETE = NO
A_FOCUSED_RECEIPTS = PASS
A_ISOLATED_ASSERTIONS_EXECUTED = NO
BLOCKING_ISSUES = 2
```

Agine的exact graph、canonical receipt和既有focused protocol/Host/service evidence可在当前
service-scoped ingress baseline上继续成立，但真实dependency-only isolated run没有进入test
assertions。当前有两个顺序blocker：target config binding ownership，以及越过该点后的shared assembly
prepare timeout。

## 1. Exact inputs

| 项 | 值 |
| --- | --- |
| A task | `P5-F445H-I7-A-agine-service-protocol-host-terminal.md` |
| Skiff acceptance baseline | `54ef44d0ed6a22f495be3509c273d24852521cf1` / `bb1a8f719e5d49db74db02164c5f0d76db209ebb` |
| Internals source base | `54286599be3d297f4f8231091f7f78ad61e2c20b` |
| Internals runtime-assembly-v3 mechanical commit | `a3f46c982b7ff92c2f3041c3791db130f193fb70` |
| Internals integrated identity at ledger time | `fb0030be1175c1cc29c572401bcd921aa9676ee3` / `3b42bd3a84aaf4862b414efdb2c8421fe4392adf` |

## 2. Passing evidence

| Evidence | 结果 |
| --- | --- |
| Agine exact service graph | PASS，exit `0` |
| Relay/AIHub dependency graph continuity | PASS，两个graph均exit `0` |
| Agine canonical receipt | PASS |
| combined T0 + service receipts | PASS，`47 passed / 2 generated-only skips` |
| Skiff fixed-profile projection exact Rust receipt | PASS |
| service-scoped ingress same-Host/same-path dispatch | PASS `1/1`；既有选择suite `12/12` |

这些结果说明旧IngressSelector Host/global collision不再阻塞A，也没有推翻此前已集成的Agine
protocol/Host focused receipts。

## 3. Blocking evidence

### 3.1 Dependency-only target config

Agine dependency-only isolated run在fixture阶段失败：

```text
missing config binding cookieName
```

当前ordinary service不会读取`config.skiff-test.yml`，而helper构造的dependency-only base assembly排除
target，因此没有target production-owner deployment提供`cookieName`。

这是新的fixture/config ownership gap；T1只关闭了dependency base-assembly identity/flag缺口，没有证明
target config已经进入dependency-only assembly。

### 3.2 Read-only includeTarget comparison

只读`includeTarget`对照可以越过`cookieName`，但随后与Relay/AIHub相同：

```text
runtime.assembly_candidate_started
HTTP 504
AssemblyActivationRejected: assembly activation prepare timed out
```

该对照只用于定位遮挡顺序。按上层约束，本次没有把`includeTarget`改成production workflow，也没有修改
20秒timeout。对照同样没有进入Agine service assertions。

## 4. Verdict and recovery

```text
A_COMPLETE = NO
A_ISOLATED_ASSERTIONS_EXECUTED = NO
```

恢复顺序：

1. 冻结ordinary service test config与dependency-only target assembly的唯一owner；
2. 不得在Agine business source复制`cookieName`或把只读`includeTarget`对照直接产品化；
3. 关闭shared activation prepare budget blocker；
4. 在同一final Internals/Skiff identities上重跑Agine isolated matrix；
5. 只有non-zero assertions与A positive/negative matrix全部GREEN后，才能写`A_COMPLETE = YES`。

本ledger只提交Skiff result文档，没有修改任何production/tests，也没有访问stable/live/network、shared
Mongo、OAuth或browser。

Agine config/fixture contract、T0/T1 tooling、Skiff assembly activation、repo identity或temporary ownership
变化会使相应证据失效。
