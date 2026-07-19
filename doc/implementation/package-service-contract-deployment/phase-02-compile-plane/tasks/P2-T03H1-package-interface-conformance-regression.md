# P2-T03H1：Package Interface Conformance Regression

## 目标

修复T03H把source exact interface owner误套到external package interface的回归：source-declared interface按
T03H exact facts验证；已由typed package facts验证的package interface继续由package interface owner处理，
不得要求或伪造source-owned identity。

## 依赖与写域

- 依赖T03H；由T03I完整lowering测试发现。
- 独占`compiler/source/src/contract_type_resolution/interfaces/**`及直接source测试；只读运行既有lowering package
  interface regression test。
- 不修改lowering、compiled/projection-input/projection、artifact schema或integration fixtures。

## 完成态

1. exact conformance builder只为source-declared interface建立`ValidatedSourceInterfaceConformance`；
   `TypeRefIr::PackageSymbol`等已验证external interface不报“no source-owned exact interface identity”，也不生成
   fake source fact。
2. unknown/invalid implements entry仍由现有type/interface resolution fail closed；不得用“全部跳过external”吞掉
   unresolved selector或签名错误。
3. source interface的ContractTypeId exact conformance、同identity不同alias、不同identity拒绝等T03H证据保持。
4. package interface local boxing恢复，package interface method/type argument仍来自canonical package facts，不被
   alias-shaped ServiceSymbol或source fallback接管。

## 聚焦验收

- source exact interface tests与新增package-interface ownership负例/正例。
- `cargo test -p skiff-compiler-lowering lowers_package_interface_box_to_local_method_table -- --nocapture`。
- source/lowering最小check、changed-file rustfmt和`git diff --check`；不运行宽gate。

## 执行合同

- DAG：波次9f窄repair；完成后解除R10I evidence refresh与production复验。风险：中；不改变设计/API shape。
- worktree：`/Users/geek/workspace/skiff-p2-t03h1-package-interface-repair`；分支：
  `codex/p2-t03h1-package-interface-repair`；从含T03I/T04C的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；修改前不跑测试或宽泛重研究。
- 提交一个聚焦commit和自验收矩阵，不push。
