# P5-F445H-I7-P3C Foreign DB compiler/source/lowering result

状态：

```text
P3C_COMPLETE = YES
P3R1_UNBLOCKED = YES
TASK_SCOPE_EXPANDED = NO
DECISION_REQUIRED = NO
```

## 1. Identity

| 项 | 值 |
| --- | --- |
| baseline commit | `83bbb5acb73fe338a6005cedb8405e7a7c0cbee2` |
| baseline tree | `4498ae78ade93372f7a3c53ae7f9fe08eeb1f2b6` |
| implementation commit | `f43b7f3c8586921a60d2dbeb6ff5fd5f8362d317` |
| implementation tree | `82130df55600100da246560844249826cd053a4f` |
| P3R0B validation dependency | `0e66bfaf14a2658eb36e28083dda68a231d2a0e1` |
| temporary join commit | `832e750d30bbef181ea6ccd988e9eaa91ee755ca` |
| temporary join tree | `4cfaf42a6efdaf39ce83ee3a2ed4e0098944dcee` |

## 2. Compiler outcome

已完成以下compiler-owned工作：

- test service driver只为带`topLevelAlias`的直接dependency从canonical artifact store加载exact
  PackageArtifact与File IR；
- source边界逐跳核验implementation symbol、type link、artifact file ref、loaded File IR、type declaration
  和DB attachment；
- 外部DB metadata只作为编译期事实进入类型检查与lowering，不写入consumer的DB declarations；
- 所有DB target surface输出主依赖alias、source symbol path与exact ABI；package requirement同时固定
  provider build；
- DB key/field类型从provider canonical File IR解析，不按consumer短名重建；
- public alias、non-DB type、缺文件/link/attachment、stale identity、同名跨artifact替换均fail closed；
  两个provider的同名`model.Session`保持不同primary alias与ABI。

没有新增File IR、PackageArtifact或Local ABI字段，也没有把provider DB declaration/schema复制到
consumer artifact。

## 3. Validation

在P3C implementation commit上通过：

```text
cargo check -p skiff-compiler-source -p skiff-compiler-lowering \
  -p skiff-compiler --tests --locked

cargo test -p skiff-compiler-source foreign_db_targets::tests \
  --locked -- --nocapture
```

结果分别为locked check成功，以及foreign DB source-validation单测`3/3`通过；P3C之外的既有
source/lowering全套失败保持baseline原状。

在P3C与P3R0B的临时exact join上通过：

```text
cargo test -p skiff-compiler --test package_imports \
  test_service_top_level_alias_lowers_foreign_db_targets_to_the_primary_dependency \
  --locked -- --exact --nocapture
# 1/1 passed

cargo test -p skiff-compiler --test package_imports --locked
# 13/13 passed
```

matrix覆盖insert/find/optional/require/update/replace/upsert/count/exists、`DbQuery`、
insert/update/delete many、claim、lease和transaction内部operation，并断言exact primary alias、
ABI/build、无consumer DB declaration复制、public alias与non-DB负例。测试还构造
`RuntimeAssembly`，验证compiler产物可以进入P3R0 linker。

Internals integration worktree中的真实`codex-relay/service-tests`也使用隔离artifact root完成
compile-only验证：依次发布std、`llm-api`、`llm-providers`和`codex-relay`，随后
`compile_package_project_for_test`成功编译`agine.ai/codex-relay-tests`。该验证没有启动MongoDB、
runtime、stable instance或网络服务，也没有提交Internals改动。

## 4. RED and upstream closure

首次full-chain的RED为`MissingDbTargetTypeDeclaration`。根因是P3R0只接受map key、declaration
symbol和export symbol完全相等且DB attachment仅为`LocalType`，与canonical compiler产物不一致。
本节点没有绕过或修改runtime；P3R0B提交按exact type index和canonical module/name匹配，并接受同一
DB对象的`DbObjectSymbol`/`LocalType`表达。上述临时join验证证明该上游修复关闭了阻塞。
