# Phase 1 live preflight: deferred Phase 0 fixes and `router-live:agine` static review

Status: 完成（修复项已提交，审查隐患清单见下；结论 checklist 供 Phase 1 首轮 Live 使用）

审查对象：

- `scripts/check-router-agine-live.mjs`（1462+ 行，Phase 0 combined managed Live selector）
- `scripts/lib/verify-live-registry.mjs`（`router-live:agine` 注册段，key `router-rust-agine-live`）
- 相关 lib：`scripts/lib/package-service-authoring.mjs`、`command-execution.mjs`、
  `cargo-target-dir.mjs`、`mongod-live-harness.mjs`（引用）
- 参考：`doc/implementation/bytecode-vm/results/phase-0.md` Known residual risks 1/2

## 1. 修复项（任务 1）：host-tools `--check` 透传失效

Phase 0 residual risk 1：phase 2 用 `npm run e2e:host-tools -- --check`，经 npm workspace
脚本（`npm run e2e:host-tools --workspace @agine/client`）透传 `-- --check` 被吞，实际执行
完整对话。

改动（`scripts/check-router-agine-live.mjs`）：

- `HOST_TOOLS_CHECK_COMMAND` 常量由 npm 形式改为直接执行形式：
  `node <agineRoot>/client/e2e/host-tools.mjs --check`（行 116-119，移到 agineRoot 定义之后；
  原常量行删除）。
- phase 2 执行改为 `runAttachedCommand(process.execPath, [host-tools.mjs 绝对路径, '--check'])`，
  绕过 npm 中间层（行 361-369），env 沿用 `hostToolsEnv()`，与 phase 3 直接 `node host-tools.mjs`
  的既有写法一致。
- manifest `phases.hostToolsCheck.command` 字段使用同一常量（行 356），与实际执行一致。
- 文件头注释 phase 2 描述同步更新（行 9-10）。

验证：

- `node --check scripts/check-router-agine-live.mjs` PASS。
- `SKIFF_ROUTER_AGINE_LIVE_INTERNALS_ROOT=/Users/geek/workspace/internals-p0-host node scripts/check-router-agine-live.mjs --preflight` PASS
  （skiff=cc41fd88，internals=4b0741e，skiff-packages=db4ddd9e）。
- 未跑完整 Live（按任务约束）。

确认：phase 2 链路已无 npm 中间层（grep `e2e:host-tools` 仅剩注释/文档引用，无执行路径）。

## 2. manifest 保留机制确认（任务 2）

机制已可用，无需改代码：

- 行 412-416：`SKIFF_ROUTER_AGINE_LIVE_MANIFEST_OUT` 未设时默认写到临时目录
  `join(tempRoot, 'router-agine-live-manifest.json')`（PASS 后随 tempRoot 被清理，即 Phase 0
  丢失的原因）；设置后写到固定路径，`finally` 清理只删 tempRoot，manifest 保留。
- 本次顺手修复（同文件）：写 manifest 前 `mkdir(dirname(manifestPath), { recursive: true })`
  （行 415），避免父目录不存在时在完整跑完后才 ENOENT 失败。

**Phase 1 用法（强制）**：

```bash
SKIFF_ROUTER_AGINE_LIVE_MANIFEST_OUT=<固定绝对路径，如 doc/implementation/bytecode-vm/results/artifacts/router-agine-live-manifest.json>
```

注意点：

- manifest 只在三段全 PASS 且 `validateAgineLiveManifest` 通过后才写出（行 398 → 412）；失败
  运行即使设置了 MANIFEST_OUT 也不产出 manifest，失败证据靠 `SKIFF_ROUTER_AGINE_LIVE_KEEP_ON_FAILURE=1`
  保留 temp workspace 与 `dumpManagedLogs` 输出。
- manifest schema 为 `skiff-router-agine-live-manifest-v1`、`engine: "legacy-tree"` 硬编码；
  Phase 1 bytecode 落地后必须 bump（见隐患 F2）。

## 3. 静态审查隐患清单

### A. 环境变量 / 路径假设

| # | 等级 | 位置 | 后果 | 建议 |
| --- | --- | --- | --- | --- |
| A1 | 高（已修） | 行 95-100；`warnUnsetRootEnv` 行 1022-1039 | `SKIFF_ROUTER_AGINE_LIVE_INTERNALS_ROOT` 未设时静默指向主工作区 `../internals`（main 分支）：Phase 1 若忘设 env，会静默测试 main 分支 internals（非 Phase 1 代码），preflight 照常通过，只在 manifest 里留下错误 commit 证据 | 已修：main() 入口打印警告（internals/skiff-packages 两个 env 未设时均警告，preflight 和完整 run 都可见）。Phase 1 checklist 仍把该 env 列为强制项 |
| A2 | 中（已修同 A1） | 行 99 | `SKIFF_ROUTER_AGINE_LIVE_SKIFF_PACKAGES_ROOT` 默认主工作区 skiff-packages（无 worktree，P0 即如此） | 已加警告；Phase 1 无 skiff-packages 改动时可接受 |
| A3 | 低 | 行 102-111；preflight 行 1042-1058 | 其余四个 SKIFF_ROUTER_AGINE_LIVE_*_ROOT 若设置但路径错误：preflight 目录检查 fail-closed，非静默。无需处理 | 保持 |
| A4 | 中（已修） | 行 412-416 | `SKIFF_ROUTER_AGINE_LIVE_MANIFEST_OUT` 父目录不存在 → 完整跑完后 writeFile ENOENT 整体失败 | 已修：写前 mkdir recursive |
| A5 | 中 | 行 47、140-141 | repoRoot 固定为脚本所在 checkout（`dirname(import.meta.url)/..`）：skiff 侧 compiler/router/runtime 永远从运行 harness 的那个 checkout 编译（cargoTargetDir(repoRoot)）。在 main worktree 跑 harness = 验证 main 的 skiff 二进制，与 internals worktree 形成"两仓不同源"证据 | Phase 1 必须从 skiff 实现 worktree（含本 harness 的 checkout）运行 harness，不能从 main 跑。写入 checklist 第 3 条 |

### B. 编译产物假设

| # | 等级 | 位置 | 后果 | 建议 |
| --- | --- | --- | --- | --- |
| B1 | 低 | 行 204-227 | 二进制来自 `skiff/build/cargo-target/debug/`（跨次运行持久化，不清理）。陈旧风险被 cargo fingerprint 抵消：router/runtime 由显式 `cargo build` 按源码指纹重建；skiff-compiler 是 smoke-fixture 的依赖（test-runner/Cargo.toml L41），authoring 的 `cargo run` 同样按指纹重建。构建失败会 captureCheckedCommand 抛错，不会静默用旧二进制。manifest 记录的 SHA 是实际二进制 SHA，证据诚实 | 无需改动。Phase 1 若改 Rust 源码，直接在该 checkout 跑即可（cargo 自动重编） |
| B2 | 低 | 行 206-216 | router/runtime `cargo build` 未带 `--locked`（与 Cargo.lock 漂移风险） | 保持（单一 workspace lock，风险可忽略） |
| B3 | 低 | 行 221-222 | compiler 二进制路径 `targetDir/debug/skiff-compiler` 在 authoring 后 access() 校验，缺失即抛错（fail-closed） | 保持 |

### C. 发布流程假设

| # | 等级 | 位置 | 后果 | 建议 |
| --- | --- | --- | --- | --- |
| C1 | 低 | 行 113-122（BUILD_ROOTS）；行 516-560（authorAgineStack 循环） | BUILD_ROOTS 顺序 packages → skiff-packages → services，配合 `isUnpublishedExactDependency`（行 732 正则覆盖 `has no published (?:provider )?(?:PackageArtifact|ServiceContract) pointer`）延后重试：新 package 首次发布也命中同一正则被 defer，依赖发布后重试成功；一轮无进度即抛错（行 545-551）。覆盖完整，非静默 | 无需改动 |
| C2 | 低 | 行 565-582 | config snapshot authoring 固定 profile 'dev'（服务只带 config.dev.yml）；Phase 1 若服务新增其他 profile 配置会 fail-closed | 保持，注释已说明 |
| C3 | 已确认（无需改） | compiler/driver/authoring/actor_routing.rs（3d468e38 merge 内容）+ be3401d8 | actor-routing projection 不再被 harness 覆盖：P0 修复（"keep compiler-authored actor routing projection"）是 main 的祖先 commit；harness 全文无对 `records/actor-routing/current.json` 的写入；compiler 侧测试 `compiler/tests/actor_routing_projection_publish.rs` 在 HEAD | 结论：P0 修复在 main 完整，Phase 1 无需重复处理 |

### D. 进程 / 端口 / mongo 清理

| # | 等级 | 位置 | 后果 | 建议 |
| --- | --- | --- | --- | --- |
| D1 | 低 | 行 434-489 | PASS 路径清理完整：children 逆序 SIGTERM→20s→SIGKILL、log 句柄关闭、ingress close、harness.cleanup()（临时 mongod）、assertPortsClosed 再 release、chmod u+w 后 rm tempRoot（read-only workspace）。清理失败 AggregateError → exit 1（fail-closed） | 保持 |
| D2 | 低 | 行 435-439 | KEEP_ON_FAILURE 仅在失败时保活（`KEEP_ON_FAILURE && runFailed`）；PASS 时即使设了也照常清理。语义清晰 | 保持 |
| D3 | 中 | 行 333-341（npm chat-smoke）、365-378（host-tools） | npm/直接 node 子进程的孙进程（smoke 内部可能 spawn 的 gateway/host 进程）不在 children 跟踪内；若 smoke 异常退出可能残留进程占端口/连接。P0 run6 assertPortsClosed 通过未见残留，但 Phase 1 每次跑完建议 pgrep 复核 | 写入 checklist 第 8 条（pgrep agine-host / 端口 45000-45999 复核）；不做代码改动（涉及 internals 行为） |
| D4 | 低 | 行 189-198（端口池 45000-45999） | 并发跑多个 live harness 会在端口池内冲突（池内独占但跨 harness 无锁） | checklist：Phase 1 期间不要并行跑其它 live selector |

### E. 依赖完整性（worktree node_modules）

| # | 等级 | 位置 | 后果 | 建议 |
| --- | --- | --- | --- | --- |
| E1 | 高（已修） | preflight 行 1064-1096 | Phase 0 踩过 worktree 缺 `@vscode/ripgrep`，此前 preflight 对 node_modules 零校验，缺失时直到 phase 3 全栈构建后才失败（浪费整轮）。现 agine 栈按包安装 node_modules（agine/、agine/client/、agine/host/） | 已修：preflight 检查三个 node_modules 目录存在 + `createRequire` 从 agine/host 解析 `@vscode/ripgrep`，缺失即 fail-fast |
| E2 | 低 | 同上 | 依赖版本漂移（worktree node_modules 与 package.json 不符的陈旧安装）不会被检测 | Phase 1 在 worktree 重新 `npm install` 后跑 preflight；不引入版本比对（机械阈值难定，留后续） |

### F. manifest 校验

| # | 等级 | 位置 | 后果 | 建议 |
| --- | --- | --- | --- | --- |
| F1 | 低 | 行 1308-1342（validatePhases） | schema 覆盖三段全部字段：chatSmoke/hostToolsCheck 各 6 字段（command/cwd/ingressBase/startedAt/finishedAt/status），hostToolsFull 15 字段（追加 runtimePid/workspace/allowedTools/sampleFile/sampleBytes/terminal/assistantChars/toolCalls/allowedToolCalls），exactKeys 拒绝未知键/缺键；与 manifestBase 构造（行 289-321、327-397）逐一吻合。覆盖完整 | 无需改动 |
| F2 | 中 | 行 1142-1144（engine 断言）、1152（schemaVersion 常量 L1151） | schemaVersion v1 + `engine` 硬编码 `'legacy-tree'`：Phase 1 bytecode 落地后，验证器会拒绝新证据（fail-closed，方向正确） | Phase 1 必须 bump `AGINE_LIVE_MANIFEST_SCHEMA_VERSION` 与 engine 断言，并更新 verify-live-registry 描述；列为 checklist 第 7 条 |

## 4. 结论：Phase 1 Live 前 checklist（必须全部满足）

1. `SKIFF_ROUTER_AGINE_LIVE_INTERNALS_ROOT` 显式指向 Phase 1 internals worktree（不依赖默认值；默认会警告并测 main 分支）。
2. `SKIFF_ROUTER_AGINE_LIVE_MANIFEST_OUT=<固定绝对路径>` 携带（父目录自动创建；仅 PASS 时产出）。
3. 从 **skiff 实现 worktree**（含本 harness 的 checkout）运行 harness——skiff 侧二进制来自 repoRoot，不要在 main worktree 跑并声称验证 worktree 代码。
4. internals worktree 已装依赖（agine/client/host node_modules + `@vscode/ripgrep` 可解析）；先 `--preflight` 通过。
5. aihub `config.dev.secret.yml` 含非空 deepseek.apiKey（preflight 检查项）。
6. 不并行跑其它 live selector（45000-45999 端口池无跨 harness 锁）。
7. bytecode 证据前先 bump manifest `schemaVersion`/`engine` 断言（F2）。
8. Live 结束后 pgrep agine-host + 复核 45000-45999 无残留（D3）；失败诊断用 `SKIFF_ROUTER_AGINE_LIVE_KEEP_ON_FAILURE=1`。
9. 不要在 harness 运行期间并行跑其它 cargo（共享/持久 target 目录文件锁排队，见仓库 AGENTS.md）。

## 附件：本阶段改动

| commit | 内容 |
| --- | --- |
| （代码 commit） | `scripts/check-router-agine-live.mjs`：phase 2 直连 node 执行 `host-tools.mjs --check`；HOST_TOOLS_CHECK_COMMAND 改直接形式；manifest 父目录 mkdir；preflight 增加 node_modules/@vscode/ripgrep 校验与 env 未设警告 |
| （文档 commit） | 本报告 `doc/implementation/bytecode-vm/results/phase-1-live-preflight.md` |
