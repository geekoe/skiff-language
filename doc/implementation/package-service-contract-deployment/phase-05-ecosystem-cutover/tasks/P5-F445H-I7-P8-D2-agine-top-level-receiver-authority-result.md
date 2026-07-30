# P5-F445H I7 P8 D2 Agine topLevel receiver authority result

状态：

```text
PASS
A1_READY_FOR_ZERO_WORKTREE_PREFLIGHT = YES
AGINE_DEFAULT_BLOCKED_BY = P8_A1
A1_RESUME_BLOCKED_BY = P8_A1_V
P8_STREAM_CAUSAL_RELATION = NONE
DECISION_REQUIRED = NO
```

## 1. Diagnostic ledger

Early diagnostic在精确Skiff
`2bcb40e61ee6b922eeca913651e2cc344a38b50e`
（tree `df2bd49666a55d73f69b63b38c267bda8d2aed9d`）和Internals
`5861c13f3a92b7fb56a5cfa689e46f5d0462a02d`
（tree `867c99c155386299e7dbb8b4fed95cee2427ba84`）上得到：

```text
Agine source declarations = 170
Agine discovered/executed = 0
```

编译在`internal/agent_bridge_host_wake.test.skiff`停止：

```text
receiver method `syncToolAttempts` on
`internal.host_toolprovider_runtime.HostCoordinator`
must resolve to a local/package executable, receiver builtin op,
or interface receiver root
```

原始sanitized日志为本机
`/Users/geek/workspace/P8-diag-agine.log`，共1236行，SHA-256：
`5fd4c79f3eb6b87b206720cffdc8e8b84a56390eef079b6081cd8be10982c5a5`。该日志只是诊断输入；
本result冻结可执行事实，不让后续任务依赖本机临时路径。

## 2. Three missing compiler links

只读源码追踪确认缺口是同一个调用跨三段没有闭合：

1. `compiler/projection/src/package_artifact/callables/mod.rs`只投影普通function，跳过
   `ExecutableKind::ImplMethod`，所以`implementationSymbols`和`callableLinks`缺少精确impl method；
2. `compiler/source/src/resolved_call_targets/builder.rs`已有local concrete、builtin、actor和interface
   receiver分支，但没有direct top-level view产生的package concrete receiver；
3. `compiler/lowering/src/function_lowering.rs`的静态receiver target对
   `TypeRefIr::PackageSymbol`返回`None`，没有把已解析target降为既有`PackageCallable`并把receiver作为
   第一项执行参数。

Runtime package-direct callable link已经可以按`PackageCallableId`执行impl method。Linker、Runtime、
test-runner协议和Agine业务代码不是当前修复owner；不能通过字符串猜测、wrapper、公开API扩大或
service boundary绕行来掩盖编译器缺口。

## 3. Frozen semantic decision

`topLevelAlias`是test-only exact implementation view。它对该view返回的精确type开放同一artifact中已有
impl method namespace；method仍写作receiver调用，不成为顶层符号。解析身份必须是：

```text
exact PackageSymbol
+ exact abiExpectation/package build
+ direct TopLevel view
+ method identity
-> existing PackageCallableId
```

generic receiver是以该`PackageSymbol`为base的`AppliedNominal`，完整type arguments参与signature
substitution。普通alias公开type、service boundary返回对象和interface receiver不获得这一权限。

## 4. DAG and gate consequence

两条工作线互不成为对方根因：

```text
P8 stream lane:   S1 diagnostic -> S2 -> S3 -> I -> X --------+
                                                               +-> J final gate
Agine compiler:   D2 -> A1 RED -> D4 -> A1-V -> A1 resume
                  -> Agine 170 resume --------------------------+
```

A1最初的直接父节点是本result；其RED揭示artifact-identity validator owner缺口后，必须先经过D4/A1-V，
再恢复A1原compiler闭环。A1-V PASS只解除A1恢复阻塞；A1 PASS才解除Agine恢复阻塞。Agine最终
`170 pass / 0 fail / 0 skip`仍由J在冻结candidate上建立，早期诊断不能冒充final PASS。

## 5. Validation

本节点为docs-only，未运行build/test/live/network/stable/Mongo。只执行：

```text
git diff --check
git grep（authority、DAG、普通alias/service/interface反向检查）
```

result提交与最终tree由handoff报告，不在本文自引用。
