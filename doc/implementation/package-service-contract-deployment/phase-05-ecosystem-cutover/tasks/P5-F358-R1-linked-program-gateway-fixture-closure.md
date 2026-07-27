# P5-F358-R1 Linked-program gateway fixture closure

状态：Ready。独立、test-only 既有 fixture 闭合。

## 直接父节点

- `P5-F358-runtime-assembly-http-gateway-linking-result.md`

F358 result 第 7.2 节已经记录
`runtime/linked-program/src/shared_image/tests.rs` 尚未把空 `RuntimeAssembly` fixture 从
`global_ingress` 迁到 required v2 `gateway_ingress`。F445G 运行 linked-program 测试时，
该既有编译错误先于新增 timeout 测试发生，阻挡验证。

## 独立复现

在没有 F445G production 改动的 base 上，用独立 Cargo target 运行：

```bash
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
```

先记录 RED：`RuntimeAssembly` 无 `global_ingress` 字段，当前字段是 `gateway_ingress`。

## 实现边界

只允许修改：

`runtime/linked-program/src/shared_image/tests.rs`

将该空 assembly fixture 的：

```rust
global_ingress: Vec::new(),
```

替换为：

```rust
gateway_ingress: Vec::new(),
```

不得修改 production、RuntimeAssembly schema、timeout、File IR、identity、其它 fixture 或 golden。
若出现第二个错误，停止并如实记录，不扩大范围。

## 验证

使用 task 专属 Cargo target：

```bash
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
cargo fmt --check
git diff --check
```

并确认：

```bash
rg -n 'global_ingress' runtime/linked-program/src/shared_image/tests.rs
```

零匹配。

## worktree 与提交

worktree：

`/Users/geek/workspace/skiff-p5-f358-r1-linked-program-fixture`

branch：

`codex/p5-f358-r1-linked-program-fixture`

base：`b6c4d7ce`，再 cherry-pick 本任务文档。

提交 test-only implementation，再只新增并提交：

`P5-F358-R1-linked-program-gateway-fixture-closure-result.md`

最终 clean。不得派子 Agent、merge/rebase/push、stable/live/network。
