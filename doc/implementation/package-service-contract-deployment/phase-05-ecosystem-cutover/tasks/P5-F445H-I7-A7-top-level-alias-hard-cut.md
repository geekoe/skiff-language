# P5-F445H-I7-A7 topLevelAlias hard cut

状态：

```text
IN_PROGRESS
```

## 1. Frozen input

| 项 | 值 |
| --- | --- |
| Skiff baseline | `5e87d1ce3c3461e5687564807afea9db4943ba46` / `c9481fc7859919199ac84e6839b07847779fce02` |
| authority | `P5-F445H-I7-D6-test-alias-diamond-authority.md` |
| retained equivalence | `P5-F445H-I7-P4-top-level-symbolic-type-canonical-equivalence-result.md` |
| foreign DB authority | `P5-F445H-I7-P3D-test-only-foreign-db-target-authority-result.md` |
| branch | `codex/p5-f445h-i7-a7-top-level-alias` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-a7-top-level-alias` |

## 2. Scope

A7只实现D6已经冻结的alias hard cut：

- 删除`access`与`PackageDependencyAccess`，旧key由strict manifest parser拒绝；
- 增加test-service-only `topLevelAlias`，并验证完整package/service alias namespace；
- 普通alias只解析`api.yml`，top-level alias只解析精确source top-level，两者无fallback；
- top-level source reference lowering回primary alias与同一exact build/local ABI；
- 一条manifest dependency仍只生成一个requirement、binding、code slot和collection projection；
- 迁移Skiff仓库内全部当前authoring与test fixtures。

本节点不修改artifact DTO、schema generation、identity generation、Runtime loader或diamond activation merge。
若实现需要其中任何一项，停止并报告scope expansion。

## 3. Verification

验收必须覆盖：

1. 同一dependency的public alias与`topLevelAlias`可同时使用；
2. 同文本symbol path由两个alias显式选择不同surface；
3. top-level ref的File IR `dependency_ref`、ABI与build仍属于primary dependency；
4. duplicate/collision/production/transitive/旧`access`/ABI-build substitution全部fail closed；
5. requirement与projection数量没有因为第二个local view增加；
6. P4 canonical descriptor等价测试继续通过；
7. compiler input/source/driver、package imports、test-runner与相关静态检查通过。

禁止stable、live、network、Mongo、OAuth、browser与push。
