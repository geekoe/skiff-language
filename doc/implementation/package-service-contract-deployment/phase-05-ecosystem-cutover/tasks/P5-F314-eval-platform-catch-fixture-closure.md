# P5-F314 Eval platform catch and legacy fixture closure

状态：Ready。

## 直接父节点

- complete consumer audit：
  `P5-F305-platform-catch-consumer-audit-result.md`
- local exception checkpoint：
  `P5-F299-runtime-local-exception-carrier-implementation-result.md`
- required linked instruction facts：
  `P5-F300-linked-exception-sites-result.md`
- completed dependencies：F309、F310/F313、F311。

## DAG位置与边界

- 节点：platform catch DAG R5；R1/R2/R3已完成。
- 与representation S0并行；不得实现`RepresentationWrap`。
- 完成后解除W2-W request consumer，并为runtime/eval combined compile提供输入。

允许：

- `runtime/eval/**`中platform projection/forwarding与直接co-located tests；
- `runtime/driver/eval/**`中仅旧exception/identity/required-site fixture closure。

禁止修改model/boundary/capability/native/linked-type-plan/service-db/request/host/transport、compiler、
artifact/std或权威文档。

## 完成标准

### 1. Platform behavior

- eval对finite `PlatformBuiltinErrorIdentity`保持exact catch；
- `std.resource.ResourceError`的旧native/string projection测试改为`None`，不得加入platform registry；
- diagnostic/opaque wrappers继续转发或保持`None`，不改变payload bytes；
- 删除eval/driver中的旧`TypeIdentity`与string builtin构造。

### 2. Request-local fixture closure

- 删除或重写依赖旧`UserException::from_typed_payload`、`from_envelope`、`.envelope()`、
  `throw_payload_actual_type`和JSON local rethrow round-trip的测试；
- replacement只断言F299 request-local `RequestException`/carrier语义：
  exact `CatchIdentity`、same-object rethrow、source/stack/correlation保持，local path不经wire；
- test-only identity使用明确`LocalExecutionTypeIdentity`或finite platform enum，不扩展platform registry；
- orphan fixture若不在module graph且只验证已删除语义，可删除；必须记录被现有F299测试覆盖的位置。

### 3. F300 fixture migration

- runtime eval/root driver中所有linked `CallIr`使用required显式`InstructionSourceSite`；
- throw/catch fixture使用required site/catch type，不保留optional/catch-all；
- synthetic site必须选择现有精确reason，不从display/source推断。

### 4. 非目标

- 不实现service envelope、InternalError、request/host wire；
- 不恢复legacy JSON exception path或compat helper；
- 不从static throw type/shape重建actual identity；
- 不改ResourceError package ownership。

## 验证owner

```bash
cargo test -p skiff-runtime-eval --lib -- --list
cargo test -p skiff-runtime-eval --lib --no-fail-fast
cargo test -p runtime --lib -- --list
cargo test -p runtime --lib --no-fail-fast
git diff --check
```

selector非零。若F302-B2 std WebSocket或尚未合入的representation generation遮挡root入口，运行最窄
owner tests并记录精确首错；不得越界修复。不运行workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f314-eval-catch-closure`
- branch：`codex/p5-f314-eval-catch-closure`
- 风险：中高，进入A5；
- 一次性Agent，5分钟内修改；提交并返回platform/request-local/site矩阵、旧路径反搜和验证；
- 不push、不承接request/host。

