# P5-F445H-I7-M Internals test-service migration result

状态：

```text
TASK_SCOPE_EXPANDED
CHECKPOINT_READY
I7_M_COMPLETE=NO
DECISION_REQUIRED=NO
BLOCKING_ISSUES=2
```

I7 M已经形成可复验的Internals checkpoint：Relay、AIHub与Agine的test root现在都是ordinary
package artifact，显式声明`kind: test`、`subject access: topLevel`与各自的test config；runner只把
dependency services放入base assembly，最终target deployment由test root自己拥有。结构与静态gate
全部通过。

但真实isolated执行揭示了两个新的Skiff production compiler缺口。Relay在top-level dependency DB
operation target lowering失败；Agine更早在direct top-level与subject-transitive package的exact type
selection失败。因此checkpoint可以交给新的Skiff production leaf继续修复和复验，但I7 M尚未完成。

## 1. Frozen inputs and checkpoint

| 项 | 值 |
| --- | --- |
| Internals baseline | `fb0030be1175c1cc29c572401bcd921aa9676ee3` / `3b42bd3a84aaf4862b414efdb2c8421fe4392adf` |
| Skiff tool candidate | `54ef44d0ed6a22f495be3509c273d24852521cf1` / `bb1a8f719e5d49db74db02164c5f0d76db209ebb` |
| checkpoint branch | `codex/p5-f445h-i7-m-test-services` |
| checkpoint HEAD/tree | `c7e531e211fec858f3e647c20a26bdc71dbaf209` / `dcf91f0243e230ea5eff03f1f00ac2d7990d325b` |

旧提交`a8cdde6`只作为迁移清单来源，没有合并到checkpoint。

## 2. Structural migration result

| service | files / cases | 结果 |
| --- | ---: | --- |
| Relay | `9 / 75` | ordinary test artifact，test-owned config与final deployment |
| AIHub | `3 / 52` | ordinary test artifact；其中`1`个live case保持`defaultRun: false` |
| Agine | `32 / 170` | ordinary test artifact，test-owned config与final deployment |

三个production root均保持：

```text
test files = 0
config.skiff-test = 0
```

三个test root均满足：

- 作为ordinary package artifact参与编译与发布；
- manifest显式声明`kind: test`；
- 对被测subject使用`access: topLevel`；
- test config由test root自身拥有，不回灌production root。

Runner的base assembly严格为dependency-only：

| target | base assembly中的service dependencies |
| --- | --- |
| Relay | 无 |
| AIHub | 仅Relay |
| Agine | 仅Relay与AIHub |

普通target service只发布`PackageArtifact`，不进入base deployment；最终target是test-owned
deployment。`includeTarget`与`config.dev`绕过均为零。

## 3. Static evidence

| 检查 | 结果 |
| --- | --- |
| shared Node tests | PASS，`27/27` |
| Agine guards | PASS，`47/47` |
| AIHub guards | PASS，`21/21` |
| diff/check | PASS |

这些证据确认checkpoint的文件布局、manifest ownership、dependency-only assembly和禁止绕过约束已经
稳定；它们不替代真实compile/link/runtime assertions。

## 4. Dynamic evidence and blockers

### 4.1 AIHub reached activation

AIHub真实compile/link成功并进入activation prepare。随后在`30s`返回HTTP `504`，执行的service test
assertions为`0`。该结果记录的是checkpoint使用的旧Skiff tool candidate行为；它不推翻结构迁移，也不
构成I7 M完成证据。

### 4.2 Relay top-level dependency DB lowering

Relay production package发布成功，但test compile在以下位置失败：

```text
compiler/lowering/src/db_lowering.rs::resolve_db_operation_target
db update subject/model.AdminSession not declared DB object
```

精确清单包含`24`个foreign subject DB type references：

- `21`个DB operations；
- `3`个普通type references，均已成功解析。

DB operation涉及的`7`个声明：

```text
AdminSession
ChatgptOauthGate
ChatgptOauthSession
RelayApiKey
ApiKeyUpstream
ChatgptSubscriptionUpstream
LlmInteraction
```

它们都来自subject的`model.skiff`。缺口是top-level dependency DB metadata没有进入DB operation
target lowering；失败发生在File IR expression emission之前，不能由fixture、test source或runner绕过。

### 4.3 Agine exact type selection

Agine更早在以下位置失败：

```text
compiler/source/src/type_resolution_model.rs::artifact_symbolic_type_index
agine.ai/agent selected type canonical.CanonicalMessagePointView descriptor disagrees with its implementation link
```

这是direct `agent` top-level dependency与subject-transitive `Agent` package type selection的真实组合。
该失败之后仍有`172`个foreign DB operations尚未到达，其中subject贡献`164`个、agent贡献`8`个。
因此Agine既不能证明DB lowering已通过，也不能用Relay的首错覆盖其独立的exact type selection缺口。

## 5. Verdict and required continuation

```text
TASK_SCOPE_EXPANDED
CHECKPOINT_READY
I7_M_COMPLETE=NO
```

下一步必须创建新的Skiff production leaf，分别或联合完成：

1. source exact type selection，使direct top-level与subject-transitive package选择同一精确实现；
2. top-level dependency DB metadata在lowering、schema、link与runtime中的闭包；
3. 使用本checkpoint重新执行Relay、AIHub与Agine的真实isolated matrices。

修复不得引入`includeTarget`、`config.dev`、复制subject DB声明、放宽assertion gate或其它绕过。只有三组
真实test assertions执行并满足各自矩阵后，才能把`I7_M_COMPLETE`改为`YES`。

本result只持久化已有证据，没有修改Skiff或Internals production/tests，没有访问
stable/live/network/Mongo/OAuth/browser，也没有push。
