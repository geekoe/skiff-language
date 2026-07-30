# P5-F231 Isolated Runtime activation startup order result

状态：完成

## 实现

- `skiff instance supervise` 新增成对的内部 startup receipt/gate：
  - supervisor 先单独启动 managed MongoDB，成功取得进程 ownership 后写 spawn receipt；
  - isolated owner 看到 receipt 后初始化 rs0，并等待 writable primary；
  - owner 生成 canonical bootstrap，向
    `router_assembly_activation_states` upsert exact activation environment，再经 workspace ownership
    校验写 activation gate；
  - supervisor 看到 gate 后并发启动 Router 与 Runtime。
- 普通 `skiff instance supervise` 未传 gate 时保持原生命周期。
- startup partial failure继续统一执行 supervisor stop、owned instance down/status、port close、lease release
  与 owned temp removal；startup failure现在也在清理前采集隔离日志证据。
- Mongo spawn、primary election、activation seed、Router/Runtime readiness均有独立阶段诊断。
  health endpoint未建立时归因 Router；Router health已建立但exact healthy replica未出现时归因 Runtime。

## 验证

- `node --test scripts/tests/isolated-test-runtime.test.mjs`
  - 32/32 PASS；
  - 断言 Mongo spawn → primary → activation seed → gate → Router/Runtime readiness；
  - failure injection覆盖每个阶段及完整cleanup surface。
- 连续三次调用真实 `runInIsolatedTestRuntime`，均打印
  `ISOLATED_STARTUP_PASS=1/2/3`，每次随后完成 supervisor/instance/port/lease/temp cleanup；
  未出现 `NotWritablePrimary` 或 unknown activation environment。
- packages integration `f32230e6`：
  - `SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f231
    CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f231/build/cargo-target npm run test:registry`
  - Registry source 5/5 PASS；
  - build为 20/20 Available、0 Package-only；
  - 真实 Registry fixtures 5/5 PASS；
  - `All selected package tests passed.`
- `pnpm --dir scripts type-check`：PASS。
- `git diff --check`：PASS。
- `pnpm --dir scripts test`：既有非本任务失败：
  `command-caller-migrations.test.mjs` 的 publish fixture缺少现行必需 `--artifact-root`，以及 status
  fixture传入无效 ownership receipt；本任务 focused tests和真实路径均通过。

## 边界

- Skiff实现基于 `6279981bba79044808ebeff99e289be4769e5ca3` 的独立 worktree/branch。
- Registry终验使用 packages integration `f32230e6`；未修改或提交 packages。
- 未操作 shared stable instance，未 push，未清理构建或验证磁盘证据。
