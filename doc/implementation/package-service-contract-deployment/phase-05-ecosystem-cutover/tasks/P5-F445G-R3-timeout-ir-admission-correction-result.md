# P5-F445G-R3 Timeout IR admission correction result

状态：`COMPLETED / REVIEW_FINDINGS_CLOSED`。

## 1. 输入、写集与提交

| 项 | commit |
| --- | --- |
| frozen implementation base | `b2995488` |
| task | `d240d532` |
| task lockfile amendment | `a80f5e38` |
| implementation | `32f60de6` |

implementation 严格落在修订后的精确写集：

- artifact executable contract 新增唯一 runtime admission 常量
  `MAX_SAFE_EXECUTION_DURATION_MILLISECONDS`；
- `artifact-model` 仅增加本地 path dev dependency `skiff-syntax`，`Cargo.lock` 只在
  `skiff-artifact-model` 的 dependency 列表增加 `skiff-syntax`；
- linker 同时约束 statement/value timeout 为
  `1..=MAX_SAFE_EXECUTION_DURATION_MILLISECONDS`；
- execution source id 必须唯一命中，且命中的 module 必须等于当前
  `FileIrUnit.module_path`；
- tail-closure corruption 保留合法 statement refs，并精确断言 closure diagnostic；其它同组
  corruption 也改为断言各自诊断。

没有修改 syntax production、IR shape/generation、compiler lowering、linked-program、
Router、eval、host、native 或其它 fixture。

## 2. Test-first 证据

第一轮只加入 artifact 跨层锁定测试，按预期 RED：

```text
error[E0425]: cannot find value `MAX_SAFE_EXECUTION_DURATION_MILLISECONDS` in module `super`
```

加入 artifact 常量后，linker 新用例形成第二轮 RED：

- 超过 safe-integer 上限的 statement timeout 被 linker 接受；
- duplicate source id 被 linker 接受；
- focused suite 为 5 PASS / 2 FAIL。

修正后的 tail-closure 用例在生产 validator 未改的情况下已经 PASS，证明测试不再被
`missing statement 1` 提前遮挡，而是真正命中：

```text
tail dependencies do not close over all prior lanes
```

## 3. Finding 闭合

| finding | 闭合证据 |
| --- | --- |
| R2-01 unsafe duration | statement/value 均接受最大合法值；均拒绝最大值加一和 `u64::MAX`，诊断包含合法区间与实际值 |
| R2-02 source owner admission | unknown、duplicate/ambiguous、foreign-module source 均 fail closed；plan 与 lane 共用同一 foreign source id 仍被拒绝 |
| R2-03 tail test masking | entry block 同步移除被迁走的 statement ref，精确断言 tail dependency closure diagnostic |

artifact test/dev dependency 将 artifact 上限与
`skiff_syntax::ast::MAX_SAFE_DURATION_MILLISECONDS` 精确锁为相等。反向搜索确认 linker
production 只引用 artifact 常量，没有第三个 duration magic number。

## 4. 验证

全部 Cargo 命令使用任务专属 target：

```text
/Users/geek/workspace/skiff-p5-f445g-r3-admission-correction/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model timeout_execution -- --nocapture` | PASS：3/3 |
| `cargo test -p skiff-runtime-linker timeout_execution -- --nocapture` | PASS：7/7 |
| `cargo test -p skiff-runtime-linker --no-fail-fast` | PASS：58/58 unit tests；0 doc-tests |
| `cargo check -p skiff-compiler --locked` | PASS；只有既有 warning |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

没有运行完整昂贵 gate、stable、live 或 network。

## 5. 生命周期

- 没有派子 Agent；
- 没有 merge、rebase、push；
- 没有操作 stable instance 或 live target；
- implementation 与本 result 分开提交；
- worktree 在 result 提交后保持 clean。
