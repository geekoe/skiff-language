# P3-T08B：Runtime Artifact Boundary Checker

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§6.2、§9、§10、§12、§14。
- 风险/验收组：中高风险 production structure gate；T09/A01覆盖最终集成。
- 当前成熟度：R03 已验收 runtime-link checkpoint；完成后形成 Phase 03 terminal structure checkpoint。
- 有效证据状态：本任务 clean commit叠加调度时 exact R03 integration checkpoint。runtime production owner、
  checker/subject registry、fixture或依赖变化会使证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：R03 PASS，可与 T07、T08A并行。
- 解锁：T09。
- branch：`codex/p3-t08b-runtime-boundary-checker`。
- worktree：`/Users/geek/workspace/skiff-p3-t08b-checker`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计。production边界无法机械判定时回报精确缺口，
  不用 broad allowlist或 known-violation ledger掩盖。

## 写入范围

独占新 `scripts/check-runtime-artifact-boundaries.mjs`、self-test fixture/subject registry、必要 verify接线及本任务
测试。不得修改任何 `runtime/**` Rust production代码、deployment/artifact、router或 test-runner。

## 完成态

1. checker扫描 runtime production owner，禁止新 loader/linker/admission路径使用 `ServiceUnit`、`PackageUnit`、
   raw `serviceAssembly`、display-name linking、source linking、raw JSON semantic linking、request-time lazy load、
   compatibility/fallback/dual-read。
2. 规则区分 production与真正 `#[cfg(test)]` module，不因目录/文件名含 test就排除 production-reachable代码。
3. self-test能分别识别 forbidden owner被改名、移动、复制、包在 helper/facade、test-only伪装，以及 checker
   subject遗漏；每类至少一个负例。
4. legacy定义若因 Phase 05 consumer暂留，只能按精确 module/call-graph boundary隔离；禁止 symbol/path通配
   allowlist或 known violation ledger。
5. checker接入 Phase 03/`verify --only runtime` 的 authoritative subject，不重复执行昂贵 runtime suite。

## 唯一验证 ownership

```bash
node scripts/check-runtime-artifact-boundaries.mjs --self-test
node scripts/check-runtime-artifact-boundaries.mjs
git diff --check
```

## 回报

提交一个 commit，回报 commit、规则/subject索引、self-test矩阵、残余 legacy隔离证据与命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
