# P5-F445H I7 P8 D4 Implementation callable validation authority result

状态：

```text
PASS
A1_V_READY_FOR_ZERO_WORKTREE_PREFLIGHT = YES
A1_RESUME_UNBLOCKED = NO
AGINE_170_RESUME_UNBLOCKED = NO
DECISION_REQUIRED = NO
```

## 1. Frozen expansion evidence

A1 RED已经证明projection应生成：

```text
implementationSymbols["api.Worker.handle"]
  -> PackageCallableId(...top-level:api.Worker.handle)
  -> exact PackageCallableSignature
callableLinks[id]
  -> same File IR executable as Worker.handle
  -> OperationCallableKind::ImplMethod
```

当该executable同时也是public-instance method时，现有
`artifact-identity/src/package_artifact/validation.rs::implementation_link_callable_scope`遍历到新的
implementation-only callable id，却只在`packageLocalAbi.publicSymbols`查找signature，稳定拒绝：

```text
implementation link executable Worker.handle targets public callable
...top-level:api.Worker.handle without a Local ABI signature
```

诊断改动已撤回；当前integration只保留A1 RED tests/fixture和scope-expansion result。

## 2. Exact validation decision

`publicSymbols`与`implementationSymbols`是互斥owner，不是fallback顺序。每个matching callable link必须：

1. 以自身精确`PackageCallableId`在两套surface中恰好找到一个`Callable`；
2. implementation callable id必须等于
   `pkg-callable:<packageId>:top-level:<sourcePath>`，不能借另一个source path或public id换owner；
3. 使用该callable的exact signature/type-parameter scope验证implementation executable signature type refs；
4. 让link的file/index coordinate属于当前implementation link，且kind与该coordinate的function/impl-method
   owner一致；
5. 以public与implementation `ImplMethod` callable的target并集闭合
   `implementationLinks.implMethods`，不再要求每个impl method target都属于public callable；
6. 保持public-instance callable与implementation-only callable各自的canonical id和identity contribution。

同一executable存在两个callable id是合法aliasing事实，不授权两个surface互相可见。signature参数表达可以
因public-instance receiver与implementation receiver surface而不同；validator只使用各自exact signature
scope，不把两条signature错误地合并成一条public signature。

## 3. Non-solutions

禁止：

- 把implementation-only id插入`publicSymbols`；
- 把impl method link改成`InternalFunction`；
- 复用public-instance callable id/signature；
- public-first或implementation-first fallback；
- 按显示path、executable coordinate或第一个同名symbol猜signature；
- 新增schema字段、identity marker或generation。

## 4. DAG

```text
D2
↓
A1 RED checkpoint（TASK_SCOPE_EXPANDED）
↓
D4
↓
A1-V artifact identity validation
↓
A1 resume
↓
Agine 170 resume
↓
J
```

A1-V不运行A1 compiler GREEN、canonical linked fixture或Agine；这些仍归恢复后的A1/J。

## 5. Validation

本节点为docs-only，未运行build/test。只执行：

```text
git diff --check
git grep（surface owner、kind、DAG、schema与stream-lane反向检查）
```

result提交与最终tree由handoff报告，不在本文自引用。
