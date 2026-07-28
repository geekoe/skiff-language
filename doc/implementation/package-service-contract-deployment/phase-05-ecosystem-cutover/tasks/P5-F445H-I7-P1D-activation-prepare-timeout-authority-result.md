# P5-F445H-I7-P1D Activation prepare timeout authority result

状态：

```text
PASS
P1D_COMPLETE = YES
P1_IMPLEMENTATION_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

P1D已经把external business request、RuntimeAssembly activation prepare与WebSocket generation release
冻结为三个独立预算域。Assembly prepare只使用Router operator配置
`activation.prepareTimeoutMs`；`requestTimeoutMs`和deployment `policy.timeoutMs`不能再触发activation
abort。

## 1. Exact input and scope

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `54ef44d0ed6a22f495be3509c273d24852521cf1` / `bb1a8f719e5d49db74db02164c5f0d76db209ebb` |
| branch | `codex/p5-f445h-i7-p1d-timeout-docs` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p1d-timeout-docs` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree由Git handoff记录；result文档不自引用自身commit identity。

实际写集只有task冻结的八个Markdown文件，没有production、config parser、tests、fixtures、Cargo、其它
repo或外部状态写入。

## 2. Frozen contract

| Budget domain | Owner and rule |
| --- | --- |
| External business request | Router `requestTimeoutMs`平台cap与deployment `policy.timeoutMs`的更早值 |
| Activation prepare | Router operator `activation.prepareTimeoutMs`；默认`120000`；正safe integer |
| Activation client | 独立deadline，严格大于prepare；默认prepare下建议`150000` |
| WebSocket generation release | lifecycle owner独立release timeout，不从其它预算派生 |

只有prepare budget到期时，coordinator才以timeout原因abort pending activation并返回504。Reject、
disconnect、CAS冲突或admission失败保持各自错误。Prepare budget不改变普通dispatch deadline，也不进入
ServiceDeployment、DeploymentPolicy、RuntimeAssembly或artifact identity。

## 3. Hard-cut effect

以下旧绑定不再具有合同效力，P1实现必须直接删除：

- 用`requestTimeoutMs`限制assembly prepare；
- 用deployment `policy.timeoutMs`限制assembly activation；
- 用business request或activation prepare budget覆盖WebSocket generation release；
- activation client与Router prepare使用相同或更短deadline；
- 为旧配置或错误路径保留alias、fallback或dual-read。

P1D是docs-only checkpoint。它没有声称production已切换，也没有运行动态验收。

## 4. Documentation evidence

| 检查 | 结果 |
| --- | --- |
| baseline identity | PASS；exact frozen commit/tree |
| authority conflict search | PASS；没有相反canonical requirement |
| write scope | PASS；仅八个Markdown文件 |
| `git diff --check` | PASS |
| changed Markdown fence parity | PASS |
| timeout-domain positive/negative search | PASS |

没有运行build/test/live/stable/network/Mongo/OAuth/browser；这些不属于P1D证据。

## 5. P1 handoff

P1实现必须在本result合流后的exact Skiff checkpoint上：

1. 增加并strict validate `activation.prepareTimeoutMs`；
2. 让activation coordinator只使用该预算；
3. 删除request/deployment timeout到activation/release的cross-wiring；
4. 把test-runner activation client deadline设为独立且严格更长；
5. 保持普通dispatch deadline与既有WebSocket release timeout语义；
6. 用聚焦正负例证明default/custom/invalid配置、timeout abort/504与非timeout错误分类。

```text
P1D_COMPLETE = YES
P1_IMPLEMENTATION_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
