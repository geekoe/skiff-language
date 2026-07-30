# P5-F358-R1 Linked-program gateway fixture closure result

状态：Completed（test-only 单点 fixture 闭合）。

## 1. Checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| base | `b6c4d7ceeec9bdafecaeaa07b554244a1761c422` | `5e5eb634c5a0b9eb8977a7fef8b3ce822193dc28` |
| task | `64612d36c5e7e3afb52b8586ad87dbae7fb351bb` | `1e90c010c107e54810a62bed5ce9301f67e06812` |
| implementation | `3ab2cabdcbbe879ceadc1bf0fa200b7030cd5baf` | `3fa2425f307141dbac6ded955a0f37265a238352` |

工作分支为 `codex/p5-f358-r1-linked-program-fixture`，worktree 为
`/Users/geek/workspace/skiff-p5-f358-r1-linked-program-fixture`。没有派发子 Agent，没有
merge/rebase/push，没有运行 stable/live 或访问 network。

## 2. 独立 RED

使用任务专属
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f358-r1-linked-program-fixture/build/cargo-target`
运行：

```bash
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
```

编译按预期唯一失败于
`runtime/linked-program/src/shared_image/tests.rs:626`：
`RuntimeAssembly` 已无 `global_ingress` 字段，available field 为 `gateway_ingress`。

## 3. 实现

只修改 `runtime/linked-program/src/shared_image/tests.rs` 的空 `RuntimeAssembly` fixture：

```rust
gateway_ingress: Vec::new(),
```

没有修改 production、schema、timeout、File IR、identity、其它 fixture 或 golden，也没有出现
第二类错误。

## 4. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-linked-program --lib --no-fail-fast` | PASS；34 passed，0 failed |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |
| `rg -n 'global_ingress' runtime/linked-program/src/shared_image/tests.rs` | PASS；零匹配 |

Cargo 测试使用上述任务专属 target。
