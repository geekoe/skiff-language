# P5-F445H I7 P8 A1-V Implementation callable scope validation

状态：

```text
READY_FOR_ZERO_WORKTREE_PREFLIGHT
BLOCKED_BY = D4_INTEGRATION
A1_RESUME_UNBLOCKED = NO
```

## 1. Parent, baseline and ownership

- 直接父节点：
  `P5-F445H-I7-P8-D4-implementation-callable-validation-authority-result.md`
- ancestry floor：
  `bc15346042a9000b0fdd9b18bbf0802e63b262c2`
  （tree `2d76f1e13c2b9bca5010fcf6346489f09a845522`）。
- dispatch时必须提供D4已集成后的精确Skiff commit/tree。
- repo：Skiff。
- integration owner：`/root/phase05_integration_steward`。
- DAG：`A1 RED -> D4 -> A1-V -> A1 resume`。

A1-V只修复artifact identity validator。完成后恢复A1原五个compiler owner，不运行Agine、不声称
`A1_COMPLETE`。

## 2. Frozen write set

production：

```text
artifact-identity/src/package_artifact/validation.rs
```

tests：

```text
artifact-identity/src/package_artifact.rs
```

不得修改artifact-model、compiler/projection、schema constants、canonical projection、runtime、linker、
test-runner或其它文件。若测试机械拆分必须新建同目录test module，先停止并报告精确原因，不自行扩大。

## 3. Stable RED

在`package_artifact.rs`以现有fixture builder构造A1 checkpoint的最小同形artifact：

- 一个public-instance method callable位于`publicSymbols`；
- 一个implementation-only impl callable位于`implementationSymbols`；
- 两个不同canonical `PackageCallableId`指向同一File IR file/index；
- 两个link的kind均为`ImplMethod`；
- `implementationLinks.implMethods`包含该exact executable；
- `callableSemanticFacts`覆盖两个id，`boundaryProjections`仍只覆盖public id。

未改production时，`package_artifact_build_identity`/`assign_package_artifact_identities`必须稳定得到A1同类
错误：implementation-only id无法在public surface找到Local ABI signature。RED必须断言结构化
`InvalidPackageArtifact`及缺失的精确callable id，不能只匹配任意失败。

## 4. Required implementation

只调整`implementation_link_callable_scope`、同文件中当前把全部implementation target等同于public
target的`validate_public_callable_link_kinds`，以及两者的最小直接helper：

1. 对指向当前implementation executable coordinate且kind可参与该link的每个callable link，按
   `callable_id`分别查询`publicSymbols`和`implementationSymbols`；
2. 恰好一个surface必须包含同id的`PackageLocalAbiSymbol::Callable`；零个、两个或非callable都拒绝；
3. 每个`implementationSymbols[sourcePath]` callable id必须精确等于
   `pkg-callable:<packageId>:top-level:<sourcePath>`；不接受public id、另一source path的id或任意别名；
4. implementation-only callable指向`implementationLinks.implMethods`时必须为`ImplMethod`，不得接受
   `InternalFunction`、`PublicFunction`或`ReceiverMethod`；
5. 使用每个exact signature的type parameters校验implementation executable signature scope；同一
   executable上的多个callable scopes必须兼容，否则拒绝；
6. method target coverage必须由指向该coordinate的public或implementation `ImplMethod` callable并集
   闭合，不再要求每个`implementationLinks.implMethods` target都必须有public callable；public-instance
   method自身仍必须有原public callable和public-instance namespace登记；
7. 保留现有public function/public-instance validation、`callableLinks`全覆盖、semantic facts、
   boundary projection和canonical identity检查。

现有implementation-only普通function继续使用`InternalFunction`并保持GREEN；第4项的kind约束只作用于
`implementationLinks.implMethods`拥有的implementation-only impl callable，不能误伤普通function。

不得把public与implementation map合并后丢失owner，也不得以public-first fallback隐藏重复或错误owner。
本任务不新增跨domain signature normalization；现有
`validate_package_callable_signature`负责Package signature本身，exact scope负责
`validate_executable_signature`中的File IR type refs。

## 5. Mandatory matrix

| case | 必须结果 |
| --- | --- |
| A1 exact shared executable | public-instance id + implementation-only impl id均验证通过 |
| implementation-only private impl | exact implementation callable、link和scope通过 |
| duplicate id | 同id同时出现在public/implementation surface时拒绝 |
| missing | callable link id在两套surface都缺失时拒绝并报告精确id |
| non-callable | exact id解析到type/const/public-instance时拒绝 |
| wrong owner | 非canonical implementation id、top-level id被塞入public surface或绑定错误surface identity时拒绝 |
| wrong target | file/index不属于exact `implementationLinks.implMethods` coordinate时拒绝 |
| wrong signature scope | executable使用未在exact callable signature声明的type parameter时拒绝 |
| incompatible aliases | 同一executable的多个exact callable type-parameter scopes不兼容时拒绝 |
| wrong kind | implementation-only impl callable使用`InternalFunction`/`PublicFunction`/`ReceiverMethod`时拒绝 |
| public pollution | GREEN artifact的implementation-only id不出现在publicSymbols/boundaryProjections |
| canonical identity | mutation任何id/signature/target/kind都会拒绝或改变build identity，不能静默等价 |

还必须保留现有`implementation_link_type_parameters_use_the_matching_public_callable_scope`、
`implementation_symbol_callable_type_parameter_scope_is_validated`和public-instance tests GREEN。

## 6. Evidence

selector可机械调整，result必须记录精确命令与非零发现数：

```text
cargo test --locked -p skiff-artifact-identity \
  implementation_only_impl_callable_scope -- --nocapture
cargo test --locked -p skiff-artifact-identity \
  implementation_link_type_parameters_use_the_matching_public_callable_scope -- --nocapture
cargo test --locked -p skiff-artifact-identity \
  implementation_symbol_callable_type_parameter_scope_is_validated -- --nocapture
cargo test --locked -p skiff-artifact-identity package_artifact -- --nocapture
cargo check --locked -p skiff-artifact-identity
cargo fmt --all -- --check
git diff --check
```

不运行A1 compiler tests、canonical source fixture、Agine 170、J gate或stream lane。

## 7. Stop conditions

若稳定RED不能由两个冻结文件建立，修复需要schema/model/identity generation变化，需要compiler改变
callable id/signature/kind，public与implementation surface必须合并，或无法保持public-instance/ordinary
alias权限隔离，立即返回`TASK_NOT_EXECUTABLE`或`TASK_SCOPE_EXPANDED`。不得通过放宽validation、跳过link、
伪造public symbol或`InternalFunction`得到GREEN。

## 8. Handoff

提交implementation与result，报告：

- branch、worktree、commit/tree和实际两文件写集；
- stable RED、GREEN与mandatory matrix；
- schema/model/canonical projection是否保持NO-OP；
- publicSymbols/boundaryProjections无污染证据；
- `A1_V_COMPLETE`与`A1_RESUME_UNBLOCKED`。

只有全部闭合才能设置：

```text
A1_V_COMPLETE = YES
A1_RESUME_UNBLOCKED = YES
AGINE_170_RESUME_UNBLOCKED = NO
```

交给`/root/phase05_integration_steward`集成与清理；不得自行写integration、merge、push、恢复A1或启动Agine。
