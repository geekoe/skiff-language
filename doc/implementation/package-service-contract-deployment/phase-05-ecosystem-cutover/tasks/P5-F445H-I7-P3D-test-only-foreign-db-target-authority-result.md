# P5-F445H-I7-P3D Test-only foreign DB target authority result

状态：

```text
PASS
P3D_COMPLETE = YES
P3_IMPLEMENTATION_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

P3D已经把test-only foreign DB target冻结为精确package symbol → package binding → provider File IR
DB declaration链。Provider File IR是DB metadata唯一事实源；linked/runtime使用artifact、file与type index
组成的目标身份，不再允许按名字或consumer副本查找。

## 1. Exact input and scope

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `b4275a48548bb21b8294d089c9108f7142609b40` / `1a34b604cf6ddf23628c7fdb0cc469aaa1a9ef4b` |
| branch | `codex/p5-f445h-i7-p3d-foreign-db-docs` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p3d-foreign-db-docs` |
| integration owner | `/root/phase05_integration_steward` |

实际写集只有task冻结的九个Markdown文件，没有production、tests、fixtures、schema constants、Cargo、
其它repo或外部状态写入。最终result commit/tree由Git handoff记录；本文不自引用自身commit identity。

## 2. Frozen contract

| 边界 | 冻结结论 |
| --- | --- |
| Source visibility | 仅`kind: test` + `access: topLevel`可访问精确subject文件顶层DB attachment |
| Consumer target | 复用`PackageSymbolRef(alias, symbolPath, abiExpectation)` |
| Build constraint | `PackageRequirement.expectedPackageBuild`约束精确implementation artifact |
| Link chain | `PackageBinding` → exact `PackageArtifactRef` → `implementation_links.types` → provider File IR type/DB declaration |
| Runtime identity | `DbObjectTargetId(PackageArtifactRef, FileIrRef, typeIndex)` |
| Metadata owner | Provider File IR唯一拥有schema/collection/key/fields/lease/index/recoverable facts |
| Coverage | 全部DB operations、`DbQuery`、lease claim/read/guard；transaction无target |
| Failure | 缺link/type/DB attachment、ABI/build mismatch、cross-artifact substitution全部fail closed |
| Generation | File IR v9、PackageArtifact v9、Package local ABI v7、ServiceContract v5均不变 |

两个dependency的module/type文本相同不是冲突：PackageArtifactRef使其身份不同；物理collection mapping
继续按各dependency edge独立投影和校验。`typeName`只保留诊断用途，不能成为lookup key。

## 3. Permission and hard-cut effect

P3实现不得：

- 让普通`access: public` dependency、transitive dependency或production service访问内部DB attachment；
- 把provider DB metadata复制进consumer File IR、PackageArtifact或linked executable；
- 按typeName、module suffix、发现顺序或全package图扫描DB declaration；
- 在缺少exact binding/link/declaration时回退到同名target；
- 为旧target shape增加dual-read、compatibility或runtime fallback。

跨package target仍操作当前test service拥有的database namespace，不构成跨service DB访问。

## 4. Documentation evidence

| 检查 | 结果 |
| --- | --- |
| baseline identity / authority conflict | PASS；exact commit/tree，无相反代际合同 |
| write scope | PASS；仅九个Markdown文件 |
| `git diff --check` | PASS |
| changed Markdown fence parity | PASS |
| positive contract search | PASS；visibility、symbol/binding、identity、metadata owner与所有DB surface均存在 |
| negative contract search | PASS；旧service-unit metadata merge authority归零，普通dependency权限未扩张 |

没有运行build/test/live/stable/network/Mongo/OAuth/browser；这些不属于docs-only P3D证据。

## 5. P3 handoff

P3 production implementation必须从本result合流后的exact Skiff checkpoint开始：

1. 为consumer DB target生成canonical `PackageSymbolRef`并沿用topLevel exact build/ABI约束；
2. 在assembly/linker中解析exact provider artifact/file/type/DB attachment；
3. 生成并贯穿`DbObjectTargetId`，删除runtime名字扫描和consumer metadata副本；
4. 覆盖read/write/query、`DbQuery`、lease claim/read/guard及transaction内operation；
5. 增加两个同module/type dependency正例与missing/mismatch/substitution负例；
6. 恢复I7 M的Relay、AIHub与Agine真实isolated matrices。

```text
P3D_COMPLETE = YES
P3_IMPLEMENTATION_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
