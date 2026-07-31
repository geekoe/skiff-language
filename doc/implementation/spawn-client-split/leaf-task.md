# 叶子任务：从 ActorClient 拆出 SpawnClient（spawn 提交半边）

## 引用链

- 直接父节点：主 Agent `/root` 派发的任务信封（`/root/spawn_client_dev`）；本任务只依据信封中“直接父节点/权威事实”一节的已确认决策执行。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/multi-agent-development.md`、`/Users/geek/workspace/skiff/AGENTS.md`。

## 任务范围（零语义变更）

把“spawn 提交”从 eval 侧 concrete `ActorClient` 拆成独立 `SpawnClient`：

- 新增 `SpawnClient`（全 workspace 未占用该名字），仅暴露 `submit_spawn`，包装同一个 `ActorCapabilityContext`。
- `skiff_runtime_capability_context::actor::ActorClient` 删除 `submit_spawn`，仅保留 actor model 操作（`get_or_create_actor` / `replace_actor` / `find_actor` / `remove_actor` / `invoke_actor`）。
- `runtime/eval/src/spawn_ops.rs`（`submit_spawn_statement`）改用 `crate::capabilities::SpawnClient` 提交。
- `ActorCapabilityApi` trait 与 `ActorCapabilityContext` 包装层的 `submit_spawn` 保持不动（host 适配器契约，wire/ABI 表面）。

### 写集

| 文件 | 操作 |
| --- | --- |
| `doc/implementation/spawn-client-split/leaf-task.md` | 本文件 |
| `runtime/capability-context/src/spawn.rs` | 新增 `SpawnClient`（仅 `submit_spawn`） |
| `runtime/capability-context/src/actor.rs` | `ActorClient` 删除 `submit_spawn` 方法 |
| `runtime/capability-context/src/lib.rs` | 声明 `mod spawn;` 并导出 `SpawnClient` |
| `runtime/eval/src/capabilities.rs` | `pub use skiff_runtime_capability_context::{...}` 增加 `SpawnClient` |
| `runtime/eval/src/spawn_ops.rs` | 改用 `capabilities::SpawnClient` |

### 禁止修改

- `ActorCapabilityApi` / `ActorCapabilityContext`（trait / 包装层）及其 `submit_spawn`——公共契约与 host 适配器实现表面。
- actor frame / `ActorExecutionFrame` / `with_actor_execution_frame`；actor model 其余命名；`agine_actor.skiff`；service-db 命名（已提交，勿动）。
- wire 帧、`spawn.submit` / `actor.*` 控制目标、序列化名、artifact 与导出表面的非新增改动。
- host 侧同名不同类型 `runtime/host/src/capability_context/actor.rs::ActorClient`（方法名为 `get_or_create` / `replace` / `find` / `remove` + `submit_spawn*`，经 `runtime/driver/capability_context/mod.rs` 公开再导出）。父节点决策未覆盖该类型；如需同步拆分，属新 DAG 节点，应由主 Agent 决策，不在本任务内猜测实现。
- 运行中兄弟 worktree（`/Users/geek/workspace/skiff-phase6`，`impl/type-ref-phase6`）的任何文件。

## 预检结论（零 worktree 只读，锚定 baseline `1c7d5795`）

1. `ActorClient` 定义位置：`runtime/capability-context/src/actor.rs:197`（concrete 结构体）；`runtime/eval/src/capabilities.rs:166` 仅为 re-export 与 actor model 使用。host 侧另有同名不同类型（`runtime/host/src/capability_context/actor.rs:29`），方法名不同，不属于父节点描述的混合体。
2. `submit_spawn` 唯一 concrete 调用点：`runtime/eval/src/spawn_ops.rs:143-144`（`ActorClient::new(spawn_context.clone()).submit_spawn(...)`）。其余 `submit_spawn` 均为 `ActorCapabilityApi` trait impl、host 侧类型或测试 fixture。
3. wire/ABI/序列化：`runtime/transport`、`router/`、`telemetry/`、`std/`、`scripts/`、`doc/architecture/`、`doc/reference/` 均无 `ActorClient` 字符串；控制目标为 `spawn.submit` 等常量，不依赖 client 名字。
4. 名字占用：`SpawnClient` 在 `/Users/geek/workspace`（含 internals、skiff-packages、兄弟 worktree，排除 target/node_modules/.git）零占用；internals / skiff-packages 无 `ActorClient` 引用。
5. 并行 ownership：唯一兄弟 worktree 为 `skiff-phase6`（compiler type-ref），与本任务文件不相交。
6. baseline `1c7d5795` 即主工作区 main HEAD；worktree 建于 `/Users/geek/workspace/skiff-wt-spawn-client`（branch `spawn-client-rename`）。

## 实现约束

- `SpawnClient` 定义于 `runtime/capability-context/src/spawn.rs`，与 `ActorClient` 同构包装 `ActorCapabilityContext`，仅暴露 `submit_spawn`。
- 不新增依赖，不改 `Cargo.toml` / `Cargo.lock`；保持现有 rustfmt 风格。
- 不做任何行为、wire、artifact 或公共契约变更；不提交 `target/`、`.skiff-instance/` 等忽略目录。

## 验证命令（证据 owner：`/root/spawn_client_dev`）

```bash
cd /Users/geek/workspace/skiff-wt-spawn-client/runtime
cargo check -p skiff-runtime-capability-context
cargo test -p skiff-runtime-capability-context
cargo check -p skiff-runtime-eval
cargo test -p skiff-runtime-eval spawn_ops
git diff --check
```

反向搜索证明：

```bash
rg -n 'submit_spawn' runtime/capability-context runtime/eval   # ActorClient 定义/方法上不再有 submit_spawn；由 SpawnClient 持有
rg -n 'ActorClient' runtime/eval/src/spawn_ops.rs              # 无
rg -n 'SpawnClient' runtime/capability-context runtime/eval    # spawn.rs 定义 + lib/eval 导出 + spawn_ops 使用
```

注：workspace 级搜索仍会命中 host 侧 `ActorClient` 的 `submit_spawn*`（见禁止修改面，属不同类型）；本任务的反向搜索证据按 capability-context / eval 表面闭合，host 侧残留作为边界外事实报告给主 Agent。

## 交接目标

完成提交后交接给集成 Agent `/root/skiff_integration`（branch、worktree 路径、commit/tree、实际写集、自验收矩阵），并通知主 Agent `/root` 状态。不 push、不合并 main。
