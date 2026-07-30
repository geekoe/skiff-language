# P5-F445H-I7-D6 Test Alias And Package Diamond Authority

状态：alias部分已实现；DB/state部分由2026-07-30 F446 hard cut取代，不得按旧mapping/state合同重发。

## 1. Parent chain and purpose

I7 M已经把测试迁移为ordinary `kind: test` service。AIHub隔离执行同时暴露两个问题：

1. 旧`access: topLevel`把dependency公开面与implementation顶层访问做成互斥模式，测试无法在同一条
   dependency上清晰使用两者；
2. test service为直接访问`llm-providers`顶层符号声明direct dependency后，与subject的transitive
   dependency形成Package diamond。F415留下的“同build多active edge一律拒绝”规则把合法菱形误判为歧义。

本节点只冻结authoring、resolution与activation合同，不修改production或tests。

## 2. Frozen baseline and ownership

| 项 | 值 |
| --- | --- |
| Skiff baseline commit | `000e50059f86b55a11e7bffd6f17b756ef8e6221` |
| Skiff baseline tree | `994192bce251bf95e64a203bdd576cd19e240382` |
| integration branch | `codex/package-service-phase-05` |
| leaf branch | `codex/p5-f445h-i7-d6-test-alias-diamond-docs` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-d6-test-alias-diamond-docs` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检核对了repository instructions、testing/static semantics、P3D、I7 M2和F415 result。
`topLevelAlias`复用现有exact build约束；stateful diamond只改变loader对重复resolved projection的判定，
不要求artifact或wire schema升级。

## 3. Frozen contract

1. 删除`access: topLevel`；旧key与旧互斥语义严格失败，不兼容读取。
2. 每个package dependency的普通`alias`始终解析该dependency的`api.yml` public paths。同一entry可选
   `topLevelAlias`，只允许`kind: test` service使用；它必须是合法唯一identifier，并与所有package/service
   alias及其它`topLevelAlias`无冲突。
3. `<top-level-alias>/<source-module-path>.<top-level-name>`解析同一direct dependency精确implementation
   source top-level。public alias与top-level alias没有fallback或precedence。
4. 两个alias属于同一dependency edge、`PackageRequirement`和`PackageBinding`。Top-level source ref在
   lowering时canonicalize回primary alias并绑定`expectedPackageBuild`，不产生第二个requirement、code slot或
   collection projection。
5. top-level权限不传递。Subject public ABI可以闭合dependency public types，但test不能因此直接访问
   transitive dependency top-level；需要时必须声明指向该provider的direct dependency，并为该entry设置
   `topLevelAlias`。
6. direct与transitive dependency可以形成Package diamond。同exact `PackageBuild`且owner-relevant facts
   相同时，合并为一个active projection和metadata owner；同一Package ID解析到不同build、logical
   collection identity缺失/重复或system encoding collision都拒绝。
7. test数据库由平台按`(testRunId, generatedTestServiceId)`派生；edge合并不能创建第二份ConfigView、
   数据库或metadata owner。`config.skiff-test.yml`只提供Package-ID scoped业务配置。
8. alias hard cut本身不升级artifact、identity或runtime wire schema；F446会为独立config snapshot与state字段
   删除执行新的schema hard cut。若alias实现发现现有schema无法表达同一edge的
   两个authoring alias或canonical projection equality，必须停止并上报，不能擅自升代。

## 4. Documentation scope

本节点只修改authority/reference/Phase I7文档及本task/result。不修改production、tests、fixtures、历史
F415/P3D/M2 result或其它repo。历史F415 result保留当时实现事实，但其中“同一build多edge一律拒绝”
不再是当前规则，以本D6合同为准。

## 5. Completion

- Reference、compiler/package architecture、DB capability与runtime topology表达同一alias和diamond规则；
- Phase I7把D6放在production实现与M reacceptance之前；
- `access: topLevel`只允许保留在明确标为历史/旧字段的文档上下文；
- Markdown fence、`git diff --check`和正反向搜索通过；
- 提交后交Skiff integration steward合流，不merge/rebase/push。
