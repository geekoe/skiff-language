# 去掉 stack / instance：每 worktree 独立进程 + 共享缓存编译

日期：2026-08-07
状态：设计定稿，待排期实现
范围：skiff 仓库 dev 工具链重构；workspace 级约定（AGENTS.md、端口表）随之更新

## 1. 背景与动机

1. **instance 是 dev-only 概念，生产无对应物**：`instance.yml` + `skiff instance up/restart/down/status/supervise`
   只服务本机 dev；生产是 per-process 部署（远端 PM2 / systemd），router 与 runtime 可能不同机器。
   "stable instance" 只是本机主 worktree 的别名，概念上误导且随重构废除。
2. **编排层竞态事故**：2026-08-07 出现双 skiff-runtime——`instance restart` 与 `instance supervise`
   的 5 秒兜底循环在 down/up 窗口并发拉起两个进程。根因：pid 文件"最后写者赢"
   （`startProcess` 不检查存活、无条件覆盖写），编排层无法自洽。
3. **编译缓存反共享**：`stack build` 强制 `CARGO_TARGET_DIR=build/cargo-target`（21GB×N），
   绕过了共享缓存 `~/.skiff-cargo-target`，每 worktree 一份完整产物，时间与空间双浪费；
   且 build 管线每个 rs 单元附带全量 `cargo test`，JS/配置改动也撞上分钟级构建。
4. **worktree 并行开发流不顺畅**：复制 `.stack` 改三个 YAML、端口错开，隔离与共享的边界模糊。

## 2. 目标形态（已定决策）

| # | 决策 | 内容 |
| --- | --- | --- |
| D1 | 编译=共享缓存+本目录拷贝 | `cargo build`（共享缓存，增量）后**拷贝**最终二进制到各 worktree `<worktree>/build/bin/`（gitignore）。main 与 worktree 一样各自持有快照，"谁构建谁拥有"，重启永远不会加载别人的二进制 |
| D2 | 每进程独立运行目录 | run dir = 配置文件 + pid + 日志。`skiff <component> <start\|stop\|restart\|logs\|status> --dir <run dir>` |
| D3 | 二进制查找 | run dir 配置 `binary:` 字段：显式路径（部署/自定义）或缺省按 `cargo metadata` 解析共享 target dir；`skiff build` 构建后把绝对路径写入 run dir 配置（或仓库级 manifest） |
| D4 | 构建命令形态 | `skiff build <component...> [--profile debug\|release]`（仓库级动作，组件作参数，支持 all）；进程命令是组件组形态（运行目录级动作，`--dir` 作参数）。两者语义不同，形态分叉合理 |
| D5 | launchd 只开机 | 每进程一个 RunAtLoad-only agent（无 KeepAlive）；日常开发由开发 agent 手动 `start/restart`（继承 shell env，无 env 注入问题） |
| D6 | 隔离约定 | 每 worktree：独立 **profile**（config.<profile>.yml 选择 + activation 命名空间 + artifact 身份）+ 端口错开 + **独立 Mongo 进程** |
| D7 | 配置即源 | 进程直接读 run dir 的 yml，无拷贝、无生成（router/runtime/watch 一律如此；watch 已先例） |
| D8 | 进程自防双启动 | router/runtime 启动时 O_EXCL pid 文件 + 存活接管（Rust 侧），任何拉起方（手动/launchd/部署 supervisor）下同 run dir 不可能双进程 |
| D9 | 哈希核对 | `build/bin/` 旁记二进制 sha256；`skiff <component> status` 显示"运行中进程 vs 当前构建产物"哈希是否一致，防"改了代码忘了重启" |

## 3. 命令设计

```
skiff build router|runtime|compiler|all [--profile debug|release]
    # cargo build（共享缓存）→ cp 到 <worktree>/build/bin/ → 写哈希文件

skiff router  start|stop|restart|logs|status --dir <run dir>
skiff runtime start|stop|restart|logs|status --dir <run dir>
skiff watch  start|stop|restart|logs|status --dir <run dir>
    # 从 run dir 的 yml 读 binary/ports/mongoUrl；pid 文件在 run dir

删除：skiff stack *、skiff instance *、instance.yml、.stack 概念
```

run dir 布局（建议放 worktree 内，如 `<worktree>/.skiff-dev/router/`）：

```
<run dir>/
  router.yml     # binary(可省略→cargo 解析), profile, ports, mongoUrl, artifactsPath
  router.pid     # O_EXCL 创建，进程自管
  router.out.log / router.err.log
```

## 4. 隔离约定（每 worktree）

| 面 | 隔离方式 |
| --- | --- |
| activation 状态 / config.<profile>.yml / artifact 身份 | 独立 profile |
| 进程 | 端口错开（router 4000/4001 偏移；runtime 无端口） |
| Mongo | **独立 mongod 进程**（各自数据目录/端口） |
| 共享面 | cargo 缓存（`~/.skiff-cargo-target`）、telemetry 消费端（4002，可选） |

Mongo 决策依据：业务库名 = `service_id` 编码（`.`→`~`、`/`→`~~`），**profile 与 mongoUrl 路径都不参与**
（`runtime/service-db/src/storage_identity.rs:20`、测试 `service_db_runtime_profile_does_not_change_database_name`）；
router 状态库名虽来自 URL 路径可隔离，但业务数据同实例必共享——因此不做"不同库名"半隔离，直接独立实例。

## 5. 改动清单

### S1 — 进程自防 pid + run dir 配置（Rust，独立可验收）

- `router/`：启动时解析 `runDir`/`pidFile` 选项（router.yml），O_EXCL 创建 pid，存在且存活 → fail closed，存在且死 → 接管；退出清理
- `runtime/`：同上（runtime.yml）
- 配置 schema：`router.yml`/`runtime.yml` 增加 `runDir`（或 `pidFile`/`logDir`）与 `binary`（可省略）

### S2 — `skiff build` + `skiff <component>` 命令族（scripts）

- `scripts/lib/cargo-target-dir.mjs` 复用：解析共享 target dir
- `skiff build`：cargo build → cp 到 `<worktree>/build/bin/` → 写 `<binary>.sha256`
- `skiff <component> start/stop/restart/logs/status`：读 run dir yml，管理进程（SIGTERM 优雅停），status 含哈希核对
- watch 适配：`skiff watch` 同族（现在 watch 是 node 脚本，无自防 pid，按 run dir 约定管）

### S3 — 删 instance / stack

- 删 `scripts/skiff-instance.mjs`、`skiff instance` 命令、instance.yml 生成（`stack-instance-spec.mjs`）、`stack build` 的进程编排侧
- `build-runtime-stack.mjs` 拆分/简化：构建与测试解耦；`.stack/` 与 `process.*`、`devHome` 概念退役
- 删除测试：`stack-instance-spec.test.mjs`、`router-instance-binary-lifecycle.test.mjs` 等 instance 相关
- 注意：`stack-instance-spec.mjs` 曾有的中间态改动（`--config` 指向 configDir）随本阶段一并废弃
- `deploy-runtime-stack.mjs` 保留（生产部署，与 instance 无关）
- telemetry：消费端独立仓库自管不变；`telemetry.endpoint`（4002）由各 run dir 配置自行决定

### S4 — launchd 只开机

- 删 `run.skiff.instance.stable`；可选新增 `run.skiff.router`/`runtime`/`watch`（RunAtLoad only，不 KeepAlive）
- 或先不引入 launchd：开机自启后置为开放问题（dev agent 手动起）

### S5 — 文档收敛

- skiff AGENTS.md：instance/stack/stable instance 表述全部替换为 `skiff build` + `skiff <component> --dir` + run dir 约定
- workspace AGENTS.md（根目录，非 git 仓库）：端口表"管理方式"列重写；"稳定 instance"表述移除
- `doc/reference/` 相关条目收敛
- 新 worktree 流程：不再"复制 .stack 改三个 YAML"，改为生成 run dir 配置（profile/端口/mongoUrl 独立）

## 6. 验收标准

- `cargo build` 走共享缓存；main 与 worktree 各自 `build/bin/` 快照互不覆盖；重启进程加载的是本 worktree 快照
- 同 run dir 手动双启第二个进程 → fail closed（O_EXCL）
- `skiff router restart --dir X` 优雅重启；`status` 显示运行中/产物哈希一致
- worktree 独立 profile + 独立 mongod：activation 状态、业务数据、端口互不干扰
- 无 instance/stack 命令与文档残留（白名单外）

## 7. 非目标

- router 多 profile/多 assembly 并发路由（"跨 worktree 共享 router 进程"方案否决，见下）
- 共享 router 进程方案：被否决——router = 单 profile 单 active assembly（`activation_state` 按 profile 单文档 + CAS），
  两 worktree 各自 activate 会互相覆盖导致对方服务 fail-closed；正确路径是"共享二进制"（共享缓存 + 各自进程）
- 多 Mongo 实例的自动编排/脚本化（用户自行启动 mongod 或复用现有脚本）
- launchd 的 KeepAlive 兜底（崩溃由 dev agent 发现并重启）

## 8. 开放问题（不阻塞排期）

- run dir 是否统一放 `<worktree>/.skiff-dev/<component>/`（推荐），还是可任意路径
- 是否需要 `skiff dev up/status` 汇总命令（一次操作多组件）
- launchd 开机自启 agent 是否本轮引入（S4 标注可后置）
- `skiff build` 的哈希文件命名与放置约定（`build/bin/<name>.sha256` 推荐）
- 旧 `.stack/` 本地目录的清理时机（迁移完成后删除，不进 git）
