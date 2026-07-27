# P5-F445G-R3 Timeout IR admission correction

状态：Ready。F445G independent review findings 闭合。

## 直接父节点

- `P5-F445G-R2-timeout-ir-independent-review-result.md`

固定 review finding：

- R2-01：linker 接受超过 JavaScript safe-integer 上限的持久 timeout milliseconds；
- R2-02：execution source site 可通过重复 source id 或 foreign module 冒充当前 File IR；
- R2-03：tail-closure negative test 被更早的 missing statement 错误遮挡。

## 完成目标

### 1. Duration artifact contract

在 artifact-model 的 executable contract 暴露唯一 runtime admission 常量：

```rust
MAX_SAFE_EXECUTION_DURATION_MILLISECONDS: u64 = 9_007_199_254_740_991
```

linker 必须从 artifact-model 复用该常量，并对 statement/value timeout 同时只接受
`1..=MAX_SAFE_EXECUTION_DURATION_MILLISECONDS`。

syntax 仍是 language literal owner，不修改其 production。通过 artifact-model test/dev
dependency 将 artifact 常量与
`skiff_syntax::ast::MAX_SAFE_DURATION_MILLISECONDS` 精确锁为相等，防止两层漂移；不得在 linker
再写第三个 magic number。

### 2. Source site owner admission

`validate_source_site` 对 referenced `source_id` 必须：

1. 恰好命中一个 `SourceMapSource`；
2. 命中的 `module_path` 与 `FileIrUnit.module_path` 精确相等；
3. 继续保留 authored site、exact offsets、正向 span 检查。

零命中报 unknown，多个命中报 ambiguous，foreign module fail closed。该规则自然覆盖 timeout、
concurrent plan 与 lane site；不增加 wire 字段，不重建 source semantics。

### 3. 精确 tail-closure test

重写现有 `tail_closure` corruption，使 executable 的 statement refs 仍合法并真正进入
`validate_concurrent_plan`。断言 diagnostic 明确包含：

```text
tail dependencies do not close over all prior lanes
```

其它 corruption 也优先断言对应错误文本，至少不能继续用一个无区分的 `is_err()` 掩盖本 finding。

## Test-first 与验收

先新增 RED，至少覆盖：

- duration 最大合法值：statement/value 都接受；
- 最大值加一与 `u64::MAX`：statement/value 都拒绝且 diagnostic 精确；
- artifact/syntax 两层上限相等；
- duplicate source id；
- foreign-module source id；
- plan/lane 使用同一个 foreign source id；
- tail closure 精确 diagnostic。

运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-r3-admission-correction/build/cargo-target \
  cargo test -p skiff-artifact-model timeout_execution -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-r3-admission-correction/build/cargo-target \
  cargo test -p skiff-runtime-linker timeout_execution -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-r3-admission-correction/build/cargo-target \
  cargo test -p skiff-runtime-linker --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-r3-admission-correction/build/cargo-target \
  cargo check -p skiff-compiler
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-r3-admission-correction/build/cargo-target \
  cargo fmt --check
git diff --check
```

## 写集与提交

只允许：

- `artifact-model/Cargo.toml`
- `artifact-model/src/executable.rs`
- `artifact-model/src/executable/timeout_execution_tests.rs`
- `runtime/linker/src/linker/execution_validation.rs`
- `runtime/linker/src/linker/file_conversion/timeout_execution_tests.rs`
- 本 result

不得修改 syntax production、IR shape/generation、compiler lowering、linked-program、Router、
eval/host/native 或其它 fixture。

worktree：

`/Users/geek/workspace/skiff-p5-f445g-r3-admission-correction`

branch：

`codex/p5-f445g-r3-admission-correction`

base：`b2995488`，再 cherry-pick 本任务文档。

提交 implementation，再只新增并提交：

`P5-F445G-R3-timeout-ir-admission-correction-result.md`

最终 clean。不得派子 Agent、merge/rebase/push、stable/live/network。
