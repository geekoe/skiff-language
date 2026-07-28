# P5-F445H-I7-D6 Test alias and stateful diamond authority result

状态：

```text
PASS
D6_COMPLETE=YES
TEST_ALIAS_IMPLEMENTATION_UNBLOCKED=YES
DIAMOND_IMPLEMENTATION_UNBLOCKED=YES
DECISION_REQUIRED=NO
BLOCKING_ISSUES=0
```

D6已经把test dependency访问冻结为“普通alias始终访问公开API，同一entry可选`topLevelAlias`访问精确
implementation顶层”，并把合法stateful diamond冻结为exact build与canonical projection完全相同时合并。

## 1. Exact input and scope

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `000e50059f86b55a11e7bffd6f17b756ef8e6221` / `994192bce251bf95e64a203bdd576cd19e240382` |
| branch | `codex/p5-f445h-i7-d6-test-alias-diamond-docs` |
| integration owner | `/root/phase05_integration_steward` |

写集只有authority/reference/Phase I7 Markdown和本task/result。没有修改production、tests、fixtures、
schema constants、历史F415/P3D/M2 result、其它repo或外部状态。最终result commit/tree由Git handoff记录，
本文不自引用自身commit identity。

## 2. Frozen alias contract

| 边界 | 结论 |
| --- | --- |
| Public access | dependency `alias`始终解析`api.yml` public paths |
| Top-level access | 同一entry可选`topLevelAlias`，只允许`kind: test` |
| Namespace | 所有package/service alias与`topLevelAlias`全局唯一、合法identifier |
| Resolution | 两套alias无fallback或precedence |
| Artifact constraint | top-level ref canonicalize回primary alias并绑定`expectedPackageBuild` |
| Graph | 两套alias仍是一个edge/requirement/binding，不复制code slot或projection |
| Transitivity | public ABI正常闭合dependency公开类型；top-level权限不传递 |
| Hard cut | `access: topLevel`严格失败，无compatibility |

因此`aihub-tests -> aihub -> llm-providers`不会让测试直接看见`llm-providers`顶层符号。测试确需访问时，
必须声明自己的direct `llm-providers` dependency及`topLevelAlias`。

## 3. Frozen stateful diamond contract

direct与transitive两条edge都保留为真实graph facts。Activation只在以下条件全部满足时合并：

- exact `PackageBuild`相同；
- resolved source→target collection mappings canonical相同；
- owner-relevant facts canonical相同。

合并结果是一个active collection projection和一个metadata owner，不是第二份数据库。以下情况拒绝：

- 同build但mapping不同；
- 不同build指向同一physical target；
- dependency projection与service root collection冲突。

`config.skiff-test.yml`仍是test activation state binding唯一来源。D6显式取代F415 result中“一stateful
build经多edge到达一律拒绝”的过度约束，但不改写该历史result及其当时测试计数。

## 4. Generation and evidence

现有模型已经分别拥有dependency alias、`expectedPackageBuild`、edge collection mapping与resolved package
link facts；D6不改变artifact、identity或runtime frame shape，因此不升级schema/wire generation。

文档验收：

```text
git diff --check                                      PASS
changed Markdown fence parity                         PASS
current authority positive contract search            PASS
legacy access: topLevel authority search               PASS (only historical/hard-cut attribution)
production/tests/schema changes                        0
```

没有运行build/test/live/stable/network/Mongo/OAuth/browser；这些不属于docs-only D6证据。

## 5. Handoff

后继实现必须：

1. hard cut manifest/parser与source resolution到`topLevelAlias`；
2. canonicalize top-level references回primary dependency alias，并保持exact build约束；
3. 修改assembly/loader对stateful diamond的canonical equality与合并逻辑；
4. 覆盖same-build/same-mapping正例及三个拒绝矩阵；
5. 迁移test manifests/sources并恢复AIHub/Relay/Agine isolated reacceptance。

```text
D6_COMPLETE=YES
TEST_ALIAS_IMPLEMENTATION_UNBLOCKED=YES
DIAMOND_IMPLEMENTATION_UNBLOCKED=YES
```
