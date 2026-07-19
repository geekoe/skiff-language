# P2-T03H2：Canonical Interface Owner Classification

## 目标

为source conformance建立显式canonical interface owner分类，修复exact source owner误接管
compiler-known `Actor` / `ErrorPayload` 的回归，同时保持source、typed package与invalid三类现有语义。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T03H、T03H1及F09D/R10I finding。
- 独占`compiler/source`中的interface owner分类、exact conformance分派及直接source测试；只读运行既有
  lowering package-interface与compiler-known std回归。
- 不修改lowering、compiled/projection-input/projection、artifact schema或integration fixtures。

## 完成态

1. interface resolution提供单一显式owner分类（名称可等价）：source-declared exact、typed package、
   compiler-known与invalid/unresolved；分类依据现有validated semantic facts，不靠字符串或失败后猜测。
2. exact conformance builder只为source-declared interface建立source exact fact；typed package与compiler-known
   明确交还各自既有owner，不生成fake source fact。
3. `Actor`、`std.error.ErrorPayload`及其短名implements恢复；unknown、非interface与错误签名继续fail closed，
   不得用blanket skip吞掉错误。
4. T03H ContractTypeId exact tests与T03H1 package-interface ownership tests保持，分类规则不在多个调用点复制。

## 聚焦验收

- source测试新增compiler-known正例与unknown/non-interface负例，并保留T03H/T03H1证据。
- 运行最小platform std compile probe、既有package-interface lowering regression、source/lowering check、
  changed-file rustfmt与`git diff --check`；不运行R10I或宽gate。

## 执行合同

- DAG：波次9h source owner checkpoint；与T04D并行，二者共同解除R10I/F09D复验。风险：高。
- worktree：`/Users/geek/workspace/skiff-p2-t03h2-interface-owner-classification`；分支：
  `codex/p2-t03h2-interface-owner-classification`；从F09D失败候选创建。
- 启动后5分钟内完成第一次实际代码修改；修改前不跑测试或宽泛重研究。若owner分类无法由现有validated
  facts表达，回报`TASK_NOT_EXECUTABLE`，不得自行引入新公共语义。
- 提交一个聚焦commit和自验收矩阵，不push。
