# Leaf task: make whole-assembly admission failures visible and non-overflowing

## Parent

`/root/runtime_admission_fix`（主 Agent 派发：定位并修复新 runtime 加载完整 dev assembly 时
admission 阶段 tokio worker stack overflow / abort）。

## Baseline

- Repo：`/Users/geek/workspace/skiff`，main @ `ff03ec0f`（主 worktree 已检出）。
- 分支：`dev/fix-runtime-admission-overflow`。
- Worktree：`/Users/geek/workspace/wt-skiff-fix-admission`，独立
  `CARGO_TARGET_DIR=/Users/geek/workspace/wt-skiff-fix-admission/target`。

## Fault facts（主 Agent 提供，调查后修正）

- 新 runtime binary（`dev-home/bin/skiff-runtime`，sha256 `c87ec34e…`）连 stable router 后
  `runtime.assembly_admission_failed stage=admission`，错误只有
  `whole-assembly activation context construction failed`；`runtime.err.log` 有历史
  stack overflow 记录（Jul 31，旧会话）。
- dev assembly `e9a18a5d…`（15 package，ThreadActor、ToolProvider/LlmClient interface、
  recoverable codec 等）此前在 stable 上反复 admission 失败；小 assembly PASS。

## 调查结论（证据见自验收矩阵）

1. 当前代码（ff03ec0f）+ 当前 artifacts 能正常 admission dev assembly：
   - 隔离 stack（动态端口 46xxx、独立 Mongo、完整 artifacts 拷贝）中，新构建 runtime
     `c2402cce` 与 stable 二进制 `c87ec34e` 都成功 admission 并注册 healthy replica。
   - 恢复 stable 三个 service DB（mongodump/restore）后仍成功 admission。
   - `c87ec34e` 与 fresh build 符号集完全一致（34441 个 T 符号逐一匹配），仅 canonical-json
     debug 路径不同；机器码一致，无行为差异。
2. stable 实例在 03:18–03:20Z 的失败是配置快照错配的瞬时状态；03:22Z 起 router committed
   tuple 更新为 generation 1 + snapshot `9afb6df8`，同一二进制已 healthy 至今。
3. 真正的可复现缺陷是**错误根因不可见**：
   - 本仓库 anyhow（1.0.10x）的 `Display` 只打印最外层 context；`reconnect_loop` 用
     `%error`，因此 `whole-assembly activation context construction failed` 背后的真实原因
     （例如 `whole-assembly service DB index provisioning failed: ... no service DB encryption
     keyring`）完全不出现。prepare/commit reject 路径已经用 `{:#}`，recovery 路径漏了。
   - `active_assembly_context` 有 4 处 `anyhow::anyhow!(error.to_string())`，Opaque wire
     payload 的 Display 为空时会把根因压成空错误，admission 链变成无内因的
     `whole-assembly activation context construction failed`。
4. 当前代码上没有可复现的 stack overflow（deep-JSON 探针在 2000 层正常；dev assembly 各
   记录 depth 远低于 tokio 64MiB worker stack 阈值）。历史 overflow 属于旧会话二进制，不在
   本修复范围内。

## 写集

- `runtime/host/src/host/lifecycle.rs`：`runtime.router_connection_error` 改用
  `{:#}` 输出完整 anyhow 链（与 `assembly_prepare_rejected`/`commit_rejected` 对齐）。
- `runtime/host/src/loader/active_assembly_context.rs`：新增 `provider_error()` 辅助函数，
  替换 4 处 `anyhow!(error.to_string())`；Display 为空时回退 `Debug`，保证根因永不消失。
- `runtime/host/src/loader/assembly_admission/tests/full_chain/db_index_provisioning.rs`：
  新增回归测试 `provisioning_root_cause_stays_visible_in_recovery_error_chain`。
- `TASK.md`：本任务合同与结果。

禁止为旧 schema 加兼容层；未改动。

## 自验收矩阵

| 项 | 结果 |
| --- | --- |
| 新回归测试（修复前） | FAIL（空 Display 根因被吞） |
| 新回归测试（修复后） | PASS |
| `cargo test -p skiff-runtime-host --lib`（相关 crate） | 见 commit message / 交接 |
| dev assembly 隔离 admission（worktree 自建 runtime） | 成功 |
| stable 二进制 `c87ec34e` 同场景 | 成功（现状 healthy） |

## 交接

- 集成 Agent：`skiff_integration`。
- 不 merge、不 push、不碰集成分支。
