# Leaf Task: 阶段 D `skiff stack` 命令族与 deploy 复制模式（dev/stack-cmd）

## 引用链

- 权威设计：`doc/architecture/profile-stack-deployment.md`
  （integration/profile-stack @ 1d4ac521 已提交），§8 是直接父节点。
- 直接父节点：阶段 D `skiff stack` 任务（主 Agent 派发；本文件记录执行合同）。
- 集成 Agent：`skiff_integration`（集成分支 integration/profile-stack，
  HEAD 1d4ac521）。
- 本分支：dev/stack-cmd，worktree `/Users/geek/workspace/skiff-stack-cmd`。

## 零 worktree 只读预检结论（基线 1d4ac521，Git 对象锚定）

1. 基线状态：integration/profile-stack HEAD == 1d4ac521；共享主 worktree 未动；
   并行 worktree（testinfra 批次、集成 worktree）与本节点写集无重叠。
2. 真实入口：
   - `scripts/deploy-runtime-stack.mjs` 仍是“渲染三个 YAML”旧逻辑（调
     `lib/runtime-stack-config.mjs` 的 render*），需要重写为 configDir 原样复制。
   - `scripts/skiff.mjs` 无 `stack` 命令；`run()` 不传 env（stack build 需要
     CARGO_TARGET_DIR，扩展 options）。
   - `scripts/lib/package-service-authoring.mjs` 提供 runCompilerAuthoring /
     runConfigSnapshotAuthoring（cargo run 调 skiff-compiler /
     config-snapshot-tooling）；`skiff assembly build` 空 deployments 会同时写
     `records/actor-routing/current.json`（project_runtime_assembly_to_store →
     write_actor_routing_projection），无需手工写 projection。
   - 唯一能生成 canonical std records 的既有 CLI 是
     `skiff-package-service-smoke-fixture --bootstrap-only`（生产引导入口禁用）。
     主 Agent 裁决（方案 1）：在 compiler 增加内部 std-seed action，复用
     `author_official_std_package` + `publish_package_artifact_records` +
     PackageArtifactPointer CAS（参照 test-runner `seed_canonical_std` 语义），
     不出现在公共 help。
   - Mongo activation state 文档结构（`_id=profile`、`revision:0`、
     `state.schemaVersion=skiff-profile-activation-state-v1`、committed generation 0）
     由 `scripts/lib/http_live_fixture.mjs::seedHttpLiveCommittedState` 确认。
   - health wire：`activeAssembly.profile/generation`、`replicas[].profile/connected/
     state`（`isolatedRuntimeHealthReady` 确认）。
3. 执行裁决记录：主 Agent 授权本节点做唯一 Rust 改动（compiler std-seed action
   及其测试）；不改设计文档；std-seed 是 §8.5“正式 compiler 生成 std records”的
   执行机制。

## 写入范围

1. Rust（仅 compiler，最小）：`compiler/driver/authoring.rs` 新增
   `seed_official_std_package`；`compiler/driver/bin/skiff-compiler.rs` 新增
   internal object `std-seed`（`--artifact-root`、`--platform-source-root`、
   `--json`，不出现在 USAGE）；`compiler/Cargo.toml` 注册
   `tests/std_seed_authoring.rs`；bin tests 同步。
2. Node：
   - `scripts/lib/stack-config.mjs`：解析/校验 configDir 五个 YAML；
     profile 一致性 fail closed；profile token
     `[A-Za-z0-9._-]{1,200}` 拒绝 `.`/`..`；remote/verify/build 字段完整；
     相对 buildRoot/cargoTargetDir 相对 skiff 仓库根解析（实现决策，本文件固化）。
   - `scripts/lib/package-service-authoring.mjs`：新增 runStdSeedAuthoring /
     stdSeedAuthoringInvocation（cargo run skiff-compiler std-seed）。
   - `scripts/lib/stack-deploy.mjs`：deploy/init/status 共用的 shell/rsync 边界与
     ecosystem 模板；`scripts/deploy-runtime-stack.mjs` 重写为薄 CLI。
   - `scripts/skiff-stack-init.mjs` / `scripts/skiff-stack-status.mjs` /
     `scripts/skiff-stack-validate.mjs` 及 lib 逻辑。
   - `scripts/skiff.mjs`：`stack` 命令分派 build/init/deploy/status/validate，
     均带 `--configDir`；usage 增加 stack 行。
   - 示例 configDir fixture：`scripts/fixtures/stack-config/`。
3. 测试：重写 `scripts/tests/runtime-stack-deploy.test.mjs`（复制模式）；新增
   stack-config / stack-commands（validate/status/init/deploy 参数与复制语义）
   测试；新增 `compiler/tests/std_seed_authoring.rs`。

## 非目标

- 不改 instance 配置、不实现 watch、不改设计文档/架构文档、不 push。
- 不做真实远端部署（最终验收阶段）。

## 实现决策（本节点固化，供 reviewer/后续节点核对）

- `build.yml` 的 buildRoot/cargoTargetDir：绝对路径原样使用；相对路径相对
  skiff 仓库根（与 build-runtime-stack.mjs 默认一致）。
- deploy 上传 buildRoot/bin 下 router/runtime 二进制（build manifest 必须含
  这两个 unit），compiler 若已构建也上传；telemetry 为 TS unit（rsync telemetry/ +
  pnpm install）。
- PM2：对每个 app 先 `delete || true` 再 `startOrReload --only <app>
  --update-env`，最后 `pm2 save`（规避 PM2 不替换旧 app script 的坑）。
- init：本地临时 artifact root 依次 assembly build（空 deployments）→
  config-snapshot（sources=[]）→ std-seed → 校验 projection record → rsync
  `--delete` 到 remoteSkiff/artifacts → ssh mongosh insert
  `skiff-router.activation_state`（重复 profile 由 insertOne duplicate key
  fail closed）→ PM2 启动 router。
- status：ssh `cat` 远端 router.yml + ssh `curl` health；config.yml /
  远端 router.yml / health activeAssembly.profile 三者一致，且 replicas 存在
  connected==true 且 state==healthy 且 profile 一致；否则 fail closed。
- validate：五个 YAML 全部存在、可解析，profile 一致，remote/build 字段完整。

## 自验收

1. `node --check` 全部改动 .mjs 文件。
2. `node --test` 新增/受影响 scripts tests。
3. `node scripts/skiff.mjs stack validate --configDir scripts/fixtures/stack-config`。
4. `cargo test -p skiff-compiler --test std_seed_authoring` 与 compiler bin tests。
5. 不做真实远端部署。

## 交接

完成后向 `skiff_integration` 报告分支、worktree、commit、写集与验证证据。
