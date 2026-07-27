# P5-F440V1 WebSocket RPC eval test-module split

状态：Ready。F440V的纯机械结构收尾；只移动`#[cfg(test)]`代码。

## 直接父节点

- `P5-F440V-websocket-rpc-typed-evaluation-result.md`

F440V production位于`runtime_websocket_jsonrpc.rs`前约460行，后约1140行全部是同一
`#[cfg(test)] mod tests`。本leaf只把tests移到同模块子文件，production字节语义与public API不变。

实现基线为`b3ca0f0e`对应的current integration tree。

## 唯一写集与目标

- `runtime/eval/src/runtime_websocket_jsonrpc.rs`
- 新建`runtime/eval/src/runtime_websocket_jsonrpc/tests.rs`
- 本leaf result

终态原文件保留：

```rust
#[cfg(test)]
mod tests;
```

tests子模块继续通过`super::*`使用private symbol。不得拆production、重命名test、改变fixture/helper、
调整断言或修改其它文件。不得派子Agent。

## 验证

先以结构断言或文件存在性证明旧inline布局不满足目标，再移动。必跑：

```bash
cargo test -p skiff-runtime-eval runtime_websocket_jsonrpc
cargo check -p skiff-runtime-eval
cargo fmt --all -- --check
git diff --check
```

Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

result记录移动前后production部分diff应为只替换test module declaration；测试count必须与F440V一致。

## 停止与交付

若移动需要改变private visibility或production API，返回`TASK_NOT_EXECUTABLE`并保留inline tests。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440v1-eval-tests-split`
- branch：`codex/p5-f440v1-eval-tests-split`
- result：`P5-F440V1-websocket-rpc-eval-test-module-split-result.md`

Implementation与result分开提交；不merge/rebase/push。
