# P5-F445H-I7-P3C Foreign DB compiler/source/lowering

状态：`IMPLEMENTATION_BLOCKED_ON_P3R0_CONTINUATION`。

## 1. Parent and baseline

本节点实现P3D冻结的test-only foreign DB compiler合同，并消费A7、P4、P3R0与D7结果。

| 项 | 值 |
| --- | --- |
| baseline commit | `83bbb5acb73fe338a6005cedb8405e7a7c0cbee2` |
| baseline tree | `4498ae78ade93372f7a3c53ae7f9fe08eeb1f2b6` |
| branch | `codex/p5-f445h-i7-p3c-foreign-db-compiler` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p3c-foreign-db-compiler` |
| integration owner | `/root/phase05_integration_steward` |

## 2. Required implementation

1. 只有ordinary `kind: test` service的直接package dependency可通过`topLevelAlias`选择provider
   顶层DB对象。公开alias、production service与transitive dependency不得获得该能力。
2. Driver必须从canonical artifact store按直接dependency的exact artifact读取provider File IR；
   source边界沿
   `implementation_symbols -> implementation_links.types[symbolPath] -> FileIrRef -> type declaration
   -> declarations.db`
   核验完整链，缺失或身份不一致时fail closed。
3. 所有DB operation、`DbQuery`、lease claim、lease read以及transaction内部operation的
   `DbTargetIr.type_ref`必须是
   `PackageSymbolRef(Dependency(primaryAlias), symbolPath, abiExpectation)`。
   `PackageRequirement.expectedPackageBuild`继续提供exact build约束。
4. Provider File IR是collection、key、field、retention、lease、index与storage metadata的唯一owner；
   consumer File IR与PackageArtifact不得复制provider DB declaration或schema。
5. 两个dependency均声明相同`model.Session`时，必须按primary alias、build与ABI保持可区分。
6. 不新增File IR、PackageArtifact或Local ABI字段，不修改artifact generation、linked-program、
   linker、Host、Eval或service-db。

## 3. Validation matrix

- 真实test service覆盖insert/find/optional/require/update/replace/upsert/count/exists、
  `DbQuery`、insert/update/delete many、claim、lease和transaction内部operation；
- 公开alias、非DB顶层类型、production与transitive访问负例；
- 缺artifact/File IR/link/type/DB attachment、陈旧build/ABI以及同名跨artifact替换负例；
- 两个dependency同名DB对象保持精确identity；
- compiler产物进入P3R0 linker并得到provider `DbObjectTargetId`；
- Internals Relay exact compile只读临时join验证，不提交Internals改动。

## 4. Scope-expansion stop

若真实full-chain暴露的问题只能通过新增artifact字段、修改artifact generation或修改P3R0/P3R1
runtime写集解决，本节点必须停止并上报，不得越界修复。上游修复合流后，由本节点在新exact baseline的
临时join上复跑full-chain，再完成result。
