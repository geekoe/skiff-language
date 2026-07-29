# P5-F445H I7 P8 D4 Implementation callable validation authority

状态：

```text
DOCS_ONLY
PRODUCTION_WRITE = NO
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-A1-top-level-alias-instance-method-closure-result.md`
- Skiff baseline：
  `bc15346042a9000b0fdd9b18bbf0802e63b262c2`
  （tree `2d76f1e13c2b9bca5010fcf6346489f09a845522`）。
- A1 RED checkpoint：
  `c05cf4beb56db29e8d441e268fb8b57a044f0852`
  （tree `67ed93c128353fb1a8f34ea4346b5dd38e6107d8`），已由当前integration保留。

## 2. Scope

本节点只澄清既有PackageArtifact validation owner并建立A1-V任务。它不修改compiler、artifact schema、
identity代际、public API、ordinary alias/service/interface权限或P8 stream lane，不运行build/test/live/
network/stable instance/Mongo/OAuth/browser。

权威闭合是：

- implementation-only impl callable继续位于`implementationSymbols`，使用既有
  `pkg-callable:<packageId>:top-level:<sourcePath>` canonical `PackageCallableId`、signature、
  `callableLinks`和`OperationCallableKind::ImplMethod`；
- validator按精确callable id在`publicSymbols`与`implementationSymbols`中唯一查找，不按executable或
  名称猜测；
- `implementationLinks.implMethods`的target coverage由public与implementation `ImplMethod` callable并集
  闭合，不再把所有impl method target等同于public surface；
- exact implementation signature的type-parameter scope用于验证对应implementation link executable；
- public-instance callable与implementation-only callable可以指向同一executable，但保持不同canonical
  id和surface owner；
- duplicate、missing、wrong owner/signature scope/target/kind全部fail closed；
- 不得把implementation-only impl callable伪装为`InternalFunction`或public symbol。

## 3. Completion

- A1-V拥有artifact-identity validation唯一production写集与完整negative matrix；
- DAG为`D2 -> A1 RED -> D4 -> A1-V -> A1 resume -> Agine 170 resume -> J`；
- A1-V PASS只解除A1，不能直接解除Agine；
- A1保留RED checkpoint和原五个compiler owner，A1-V不吞并compiler实现；
- J显式等待A1-V与恢复后的A1；stream lane保持原样。

执行结果：
`P5-F445H-I7-P8-D4-implementation-callable-validation-authority-result.md`。
