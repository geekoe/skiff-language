# P5-F440L Runtime HTTP gateway current fixture repair result

状态：`COMPLETED`。未触发 `TASK_SCOPE_EXPANDED`。

本 leaf 只把 Runtime HTTP gateway 测试 fixture 适配到当前
`DeploymentGatewayEntry.handler: Option<PackageCallableId>`。HTTP callable entry 必须存在
handler；缺失时 fixture 以明确的不变量消息失败，不回退到 selector、空字符串或其它 callable。
production、artifact schema、compiler、fixture authoring和既有断言语义均未改变。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 精确 integration 输入 | `1897df3f98c6ba1d4f8522c5003e295380ed54e3` | `483ba35924a46bf6e08052c6fb4a7c61e113535d` |
| task worktree 起点 | `6e43c531a1d8107d5bd84f99642a7649e7b1d54b` | `cf70364eb699e78f1a3294c5d6d0fcf69bdc9eaf` |
| implementation | `a93e43507e96633c89fc86e8302aef870ae1bc49` | `f75d978463253f064fccc130b0b171804f9ec372` |

task 起点相对精确 integration 输入包含父节点合流与 F440K/F440L 调度。implementation 精确修改：

- `runtime/eval/src/runtime_http_gateway/tests.rs`

除此之外只新增本文 result。

## 2. Red 与最小修复

修改前原样执行：

```bash
cargo test -p skiff-runtime-eval --lib
```

结果为 exit 101，首个且唯一编译错误：

```text
runtime/eval/src/runtime_http_gateway/tests.rs:384:50
error[E0599]: no method named `as_str` found for enum
`Option<PackageCallableId>` in the current scope
```

精确代码 diff 只有 handler 解包：

```rust
handler: self.callable(
    entry
        .handler
        .as_ref()
        .expect("HTTP gateway fixture entry requires a handler")
        .as_str(),
),
```

因此 resolver 收到的仍是 exact `PackageCallableId`；`pre`、`guard`、selector和其它 fixture
行为完全不变。

## 3. Green 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --lib` | PASS：207 passed，0 failed，0 ignored |
| `cargo test -p skiff-runtime-eval --test catch_fixture_closure` | PASS：4 passed，0 failed，0 ignored |
| `cargo check -p skiff-runtime-eval` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

编译输出只有仓库已有 warning，没有新增 error 或 test failure。没有运行 live、instance、stable、
Router或完整 verify。

## 4. Scope 与交付状态

- 代码写集只有任务指定的单个测试文件；没有修改任何 production、其它 test、task或权威设计。
- 缺失 handler 的失败消息明确为
  `HTTP gateway fixture entry requires a handler`，不存在 fallback。
- 没有派子 agent，没有 merge、rebase、push、stable watch或 live 操作。
- implementation 与 result 分开提交；result commit/tree由交付消息记录。
