# P5-F330 Service error test-effect consumer

状态：Ready。

## 直接父节点

- R0 frozen API acceptance：
  `P5-F327-service-error-core-independent-acceptance-result.md`
- current real-path/consumer audit：
  `P5-F319-service-error-channel-delta-audit-result.md`

本任务实现F319的R3。R0 API已冻结；不得修改core、ordinary dispatcher或stream carrier。

## DAG与并行边界

- 与F328 ordinary/ingress及F329 async/stream/cancel并行。
- 本任务只迁移`ContractOperation` service effect；`PackageCallable`继续是同request package-local error。
- 完成后解除T2的W2-R service半边；host-boundary exact kind仍由W2-W拥有。
- 证据基线：worktree创建HEAD。R0 API、test effect target/outcome或dispatch ordering变化会使证据失效。

## Production写入范围

- `runtime/eval/src/test_effect_registry.rs`
- `runtime/eval/src/eval_context.rs`

测试只限：

- `runtime/eval/src/test_effect_registry.rs`内单测；
- `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs`。

禁止修改ordinary/dispatcher、async/stream/cancel、capability、R0 core、test-runner/compiler/artifact、host/
request/transport/router/std及权威设计。

## 完成标准

- `ContractOperation` throw不再把setup heap的carrier/local `TypeAddr`直接deep-clone进caller；
- registry保留actual setup payload/heap所需typed snapshot，由eval以synthetic provider身份调用同一R0
  export，再在caller heap/call site调用同一R0 import；
- exact protocol/operation target保持；不能用target string、Package-shaped target或message判断service；
- public/dependency error、private/nonclosed/encode failure、platform/Internal与opaque forward均服从R0；
- imported service effect每次得到caller-local新stack和安全RemoteBoundary，setup source/stack不泄露；
- `PackageCallable` throw继续使用local `materialize_local_test_throw`，不要求public schema、不调用service
  encoder，现有T1 exact catch/rethrow保持；
- 当前没有exact host-boundary kind时明确fail closed或不拦截，向W2-W暴露typed requirement；不得把它当
  ContractOperation或PackageCallable猜测。

## 探针

至少覆盖：

- service effect public typed exact catch；
- private/nonclosed/encode failure→一次Internal；
- imported/opaque raw forward和per-hop stack；
- setup heap销毁后caller无handle/TypeAddr别名；
- PackageCallable local negative证明未调用R0；
- wrong protocol/operation、Package-shaped/host-like target fail closed；
- sequence consume/finalize与随后response行为不回归。

```bash
cargo test -p skiff-runtime-eval --lib test_effect_registry -- --list
cargo test -p skiff-runtime-eval --lib test_effect_registry --no-fail-fast
cargo test -p skiff-runtime-eval --lib source_inline_service_effect_sequence_typed_throw_is_caught_then_responds --no-fail-fast
cargo check -p skiff-runtime-eval --lib
git diff --check
```

source-inline selector当前可能先被既知generic WebSocket public-schema决策遮挡；若如此，新增最窄linked
fixture证明service effect路径并记录遮挡，不修改WebSocket/compiler。不得运行完整eval/workspace/root/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f330-error-test-effect`
- branch：`codex/p5-f330-error-test-effect`
- 风险：高，test boundary parity；新的一次性Agent，5分钟内先分开service/package throw路径；
- 提交并返回service/package/heap/opaque/stack/sequence矩阵；
- 不push、不承接R4/W2-W或验收。

