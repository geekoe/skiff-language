# P5-F204 Module constant callable summary result

状态：完成 compiler 语义补强；Relay 根因前提已纠正。

## 实现

callable-effects 不再把所有未绑定 `Identifier` 无条件视为 `Constant`：

- 从 parsed production sources 建立模块常量身份表；
- 字面量常量及同模块常量别名链得到精确 `Constant`；
- unsupported initializer 与循环常量保持失败关闭；
- 未解析全局值产生 unknown provenance；
- 已解析的 local/native/package/contract 非 receiver callee 只在精确 call target
  的语法 callee 位置作为常量地址读取，first-class value 路径仍失败关闭。

因此零参数函数不会因为参数个数而被猜成常量；只有函数体实际返回精确模块常量时，
其 local-call summary 才传播 `Constant`。该调用不引入 unknown target、caller alias、
same-heap、write、escape 或 suspension。

## 测试

新增正负覆盖：

- 模块字符串常量 -> 零参数 helper -> 跨模块 `root.model` 调用 -> fresh record；
- unsupported constant initializer；
- cyclic constants；
- unresolved global；
- 非常量零参数函数。

结果：

- `cargo test -p skiff-compiler-source --lib --no-fail-fast`
  - `241 passed / 0 failed`
- `cargo check --workspace`
  - 通过
- `git diff --check`
  - 通过

## Relay 根因纠正

使用 fresh canonical artifact store：

1. bootstrap `skiff.run/std@1.0.0`；
2. publish `agine.ai/llm-api@0.1.0`；
3. publish `agine.ai/llm-providers@0.1.0`；
4. build `/Users/geek/workspace/internals-p5-f188/codex-relay/service`。

最新真实 artifact 为：

```text
skiff-package-build-v4:sha256:041341c5845d19e14c911736d6cf55c3479cf917d9cd0b2c0f05a0fc804b003e
```

精确 compiler 内部 facts 证明任务输入中的 Relay 归因不成立：

```text
model.upstreamKindApiKey
  effects: no effects
  returnOrigins: Constant
  unknown: none

upstream_sources.apiKeySourceView call targets
  model.upstreamKindApiKey
  upstream_health.upstreamStatus
```

`upstreamKindApiKey()` 在本任务之前已经是精确 `Constant`，callee ID 也没有丢失。
真实污染来自独立的 `upstream_health.upstreamStatus`：

```text
invokesUnknownTarget: true
unknown: UnknownCallTarget
requiresSameHeapIdentity parameters: 2, 3, 4
escape lanes: Native, External
```

它继续向 `apiKeySourceView -> upstream_sources.adminState -> relay.adminState ->
admin_http.adminState` 传播。因此 canonical deployment 仍精确停在：

```text
ingress operation adminState is boundary unavailable
```

本任务未修改 Relay 源码，也未声称模块常量修复了该 blocker。`upstreamStatus` 的 exact
binding/transfer 由后续独立任务处理。

## 不变量

- 没有 Relay/package-name 特判。
- 没有把任意零参数函数当作常量。
- unknown callee、unsupported/cyclic constant 和 first-class callable value 继续失败关闭。
- 未操作 shared stable instance，未 push。
