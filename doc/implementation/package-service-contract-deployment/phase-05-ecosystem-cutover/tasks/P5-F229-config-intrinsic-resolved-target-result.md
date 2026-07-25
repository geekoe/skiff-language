# P5-F229 Config intrinsic resolved target result

状态：完成。

## 实现

`ResolvedCallTarget` 新增严格序列化的 `ConfigIntrinsic` 目标，并携带
`require`、`optional` 或 `has` 精确种类。目标解析只识别以下直接 canonical
语法，并在 native、dependency、local callable 查找之前完成：

- `config.require<T>(...)`
- `config.optional<T>(...)`
- `config.has(...)`

局部 `config` 绑定、别名/间接调用、未知方法以及不符合上述形状的调用不会被归类为
config intrinsic，原有 config usage 校验继续 fail closed。

callable-effects 已删除重复的 AST 快速判断，改为只消费类型化目标，并将其结果建模为
Fresh、无 effect。该目标不产生 source callable key 或调用图边；compiled projection
也不会把它发布为 external/unknown `CallableTargetFact`。lowering 的挂起分析明确将
config intrinsic 视为非挂起调用。config requirement 收集、校验、lowering 和 Runtime
协议没有改变。

## 正负验收

新增和扩展的测试证明：

- require、optional、has 三种调用均解析成精确 `ConfigIntrinsic`；
- 三者均为 Fresh、无 effect，且不生成 Unknown target；
- public config caller 的 boundary projection 为 Available；
- typed require/optional requirement 仍原样保留在 callable 和 package runtime
  requirements 中；
- config intrinsic 不进入 artifact `resolvedCallTargets`；
- 严格 wire 会拒绝缺失 kind、未知 intrinsic（例如 `get`）和额外字段；
- config 别名/间接调用、未知方法和局部 shadow 继续被拒绝。

## 真实 Relay 验收

使用当前 compiler、`/Users/geek/workspace/internals-p5-f188` 的真实 Relay
源码和隔离 artifact store `/tmp/skiff-f227-relay.iB6dOG` 重新生成 artifact；
没有使用 shared stable instance。新 Relay package build 为：

```text
c4210a1c8c759921fb8b23b1cbddb51281ef40622e2fe3cd8b1871622e6409af
```

artifact 仍精确包含 17 个公开 callable。`v1Proxy` 的
`resolvedCallTargets` 已不再包含 preorder 204 的 Unknown；当前只剩 preorder
189 的精确 package direct target：

```text
agine.ai/llm-providers:chatgptPlan.responses
```

因此 config optional 污染已经消失。Relay 当前 17 个 boundary projection 中
14 个 Available、3 个 Unavailable。`v1Proxy` 仍由独立的后续语义问题保持
fail closed，其聚合事实为：

```text
invokesUnknownTarget: true
writesCallerReachable: true
throwsCallerAlias: true
requiresSameHeapIdentity: true
escapesCallerValue: true
maySuspend: true
returnsCallerAlias: false
provenance: Unknown(UnsupportedControlFlow)
```

这些剩余事实不再归因于 `config.optional`。

## 验证

通过：

- `cargo test -p skiff-compiler-source --lib --no-fail-fast`：266 passed；
- `cargo test -p skiff-compiler --test config_shape --no-fail-fast`：6 passed；
- `cargo test -p skiff-compiler-compiled --lib --no-fail-fast`：3 passed；
- 真实 Relay isolated artifact 重建及 17-callable/target facts 检查；
- `cargo check --workspace`；
- `git diff --check`。

没有修改 boundary eligibility 对 Unknown 的处理，没有 push，也没有操作 stable。
