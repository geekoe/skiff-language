# P5-F445H-I7-M3 Internals top-level alias result

状态：

```text
PASS
M3_COMPLETE=YES
TOP_LEVEL_ALIAS_MIGRATION=CLOSED
D7_409_CLOSED
M_STATUS=PARTIAL
I7_M_COMPLETE=NO
DECISION_REQUIRED=NO
BLOCKING_ISSUES=3
```

M3已经把Internals三个test service从旧`access: topLevel`迁移到A7冻结的同entry双alias模型，并使用
D7实现的stateful diamond合并规则完成真实复测。Authoring迁移与旧AIHub
`multiple active collection projections` 409均已关闭。

三个isolated run随后分别到达既有或后续Skiff compiler/linker blocker，因此总体M仍为partial，I7 M
不能标为完成。

## 1. Frozen inputs

| 项 | 值 |
| --- | --- |
| Skiff candidate | `83bbb5acb73fe338a6005cedb8405e7a7c0cbee2` / `4498ae78ade93372f7a3c53ae7f9fe08eeb1f2b6` |
| Internals baseline | `a76ee6146d650c3317c9ffef2223fdd00d7e74fe` / `ad181f9305967590fd3d5ec0c2d1bd1ab7246280` |
| Internals implementation HEAD/tree | `9327d41267cc0e79cc30e4eb45c6975e9cf6a286` / `b289796cc8dc0d1c1aab05445711aa8ae7f9d7df` |

记录本ledger时，Internals implementation已经clean交接，其 integration merge identity仍等待对应
steward回报。本result不把尚未回报的merge commit当作冻结输入。

## 2. Authoring migration

三个test root删除了全部`access: topLevel`。Subject dependency继续使用primary `alias: subject`，并在
同一个dependency entry增加：

| test service | implementation-only aliases |
| --- | --- |
| Relay | `subjectImpl` |
| AIHub | `subjectImpl` |
| Agine | `subjectImpl`、`agentImpl` |

引用规则已经逐项迁移：

- `api.yml`公开API引用继续使用primary alias；
- implementation source top-level引用才使用对应`Impl` alias；
- Agine公开type改用canonical dot syntax `agent.<public-path>`；
- Agine公开call仍使用既有slash call syntax；
- 所有`llmProviders`引用均已对照其`api.yml`确认属于public surface，因此没有新增
  `llmProvidersImpl`。

每个primary alias与对应`topLevelAlias`仍是一个manifest dependency entry。没有重复edge、
`PackageRequirement`、binding、code slot或collection projection authoring，也没有修改dependency
closure、config、collection mapping或test case。

结构规模保持：

| service | files / cases |
| --- | ---: |
| Relay | `9 / 75` |
| AIHub | `3 / 52` |
| Agine | `32 / 170` |

## 3. Static and structural evidence

| 检查 | 结果 |
| --- | --- |
| shared structural RED | 旧manifest真实失败，`7/8` |
| shared structural GREEN | PASS，`27/27` |
| AIHub workflow guards | PASS，`21/21` |
| Agine architecture checks | PASS |
| Internals旧`access: topLevel`反向搜索 | `0` |

这些证据关闭M3 authoring与structure scope，但不替代真实compiler/link/runtime执行。

## 4. Real isolated evidence

### 4.1 Relay

Relay真实isolated run越过manifest与alias迁移，停在P3 foreign DB compiler blocker：

```text
subjectImpl/model.AdminSession is not a declared db object in File IR unit expression
```

这是foreign top-level DB target进入File IR之前的compiler/lowering缺口，不是M3 authoring回归。

### 4.2 AIHub

AIHub真实isolated run不再出现D7之前的：

```text
multiple active collection projections
```

它已经越过stateful diamond admission，随后在activation link阶段失败：

```text
MissingDbTargetTypeDeclaration
```

后续P3证据把该错误精确归因到provider File IR/linker representation差异：type declaration map key为
`Session`，declaration symbol为`model.Session`，implementation type link symbol为`Session`，DB
attachment使用`DbObjectSymbol(model.Session)`。当前runtime exact DB target validator仍要求
key、declaration symbol与export symbol文本相同，并只接受`LocalType(index)` attachment。该分类属于
P3 compiler/runtime exact-identity闭包的后续owner；M3不修改或绕过它。

### 4.3 Agine

Agine先修正公开type的canonical dot syntax，随后到达现有canonical-equivalence blocker：

```text
agentImpl internal callable local:
  thread.UserMessageInput
  thread.RunReceipt
  thread.ToolResult*

primary alias PackageSchema:
  agent.thread.*
```

这是implementation view local type与primary public PackageSchema identity的闭包问题，交给后续P4
canonical-equivalence owner处理。M3没有把public引用改写为implementation引用，也没有增加
`llmProvidersImpl`绕过。

三个run均以明确的非零失败状态和精确首错结束；它们证明M3迁移已经越过对应旧authoring/D7 blocker，
不证明service test assertions或I7 M完整矩阵已经通过。

## 5. Verdict and handoff

```text
M3_COMPLETE=YES
TOP_LEVEL_ALIAS_MIGRATION=CLOSED
D7_409_CLOSED
M_STATUS=PARTIAL
I7_M_COMPLETE=NO
```

后续必须由Skiff P3/P4 production owners关闭上述compiler/linker identity gaps，并在同一最终
Skiff/Internals identities上重新运行Relay、AIHub与Agine isolated matrices。不得恢复
`access: topLevel`、复制dependency edge、修改closure/config/collection mapping或改变test case来绕过。

本result只修改Skiff integration ledger，没有修改Skiff工具代码，也没有访问
stable/live/network/Mongo/OAuth/browser或push。
