# sharded-test-orchestration: skiff test 通用编排（分片 / plan / 增量 store / 自动基线）

状态：Ready（待实现）。

## 1. 背景与问题

现状（internals 侧）：

- `internals/scripts/run-service-tests-sharded.mjs` 承担 service 测试编排：canonical fixture（全量发布
  internals 源码图到临时 store + 部署 receipts + assembly/config snapshot）+ 测试文件发现 + case 计数 +
  分片 + 并行 spawn `skiff.mjs test` + 失败日志。
- `skiff.mjs test` 只是单一引擎：接受包目录或显式文件、`--artifact-root`、成对
  `--base-assembly/--base-config-snapshot`，无发现、无分片、无并行。

问题：

1. 分片/并行/失败日志/计划预览是**通用基础设施**，却住在 internals 的 wrapper 里；skiff 作为引擎侧
   没有这些能力。
2. 粒度与成本脱节：跑单个测试文件也要全量重发整个 fixture（~60s 发布 + 全量编译），开发循环被
   拖慢；没有"少量修改 → 细粒度"的路径。
3. 用户无法在运行前得知本次是 hermetic 全量还是复用旧 store、会重发哪些 source。

目标：把编排能力下沉为 `skiff.mjs test` 的通用能力；internals 只保留"源码图清单"这一数据事实；
删除 `run-service-tests-sharded.mjs`。

## 2. CLI 设计

```text
skiff test <package-root-or-file>... --artifact-root <dir>
  [--base-assembly <identity> --base-config-snapshot <identity>]
  [--sources <manifest.json>] [--fresh] [--plan]
  [--shards <n>] [--max-cases <n>]
  [--deny-skips] [--require-tests] [--live ...]
```

新参数语义：

| 参数 | 语义 |
| --- | --- |
| `--sources <manifest.json>` | 源码清单（见 §3）。提供后执行增量发布：只发布 stale 的 source 进 store。 |
| `--fresh` | 强制全量发布（忽略 store 复用），即使 store 已存在。门禁用。 |
| `--plan` | 干跑：打印完整计划（store 模式 / 发布清单 / 测试范围 / 基线来源），不发布、不编译、不跑测试，exit 0。 |
| `--shards <n>` | 并行分片执行。目录 root 递归发现 `*.test.skiff` 并按 case 数分片；文件 root 直接作为待跑文件。 |
| `--max-cases <n>` | 每个测试激活的 case 上限（透传为子进程 env `SKIFF_TEST_MAX_CASES_PER_ACTIVATION`）。 |

未提供 `--shards` 时保持现有单进程语义；未提供 `--sources` 时不做发布；`--plan` 与
`--base-assembly` 成对校验、`--live` 校验等现有规则不变。

## 3. Source manifest（internals 生成的数据）

`--sources` 指向一个 JSON 文件，由 internals 从 `canonicalSourceDefinitions` 生成（见 §8），
skiff 引擎只消费数据、不持有清单：

```json
{
  "packages": [
    { "coordinate": "skiff.run/std",       "root": "/abs/...", "version": "0.1.0", "bootstrap": true },
    { "coordinate": "agine.ai/llm-api",    "root": "/abs/...", "version": "0.1.0" },
    { "coordinate": "agine.ai/llm-providers", "root": "/abs/...", "version": "0.1.0" },
    { "coordinate": "agine.ai/agent",      "root": "/abs/...", "version": "0.1.0" }
  ],
  "services": [
    { "coordinate": "skiff.run/account",   "root": "/abs/...", "version": "0.1.0" },
    { "coordinate": "agine.ai/aihub",      "root": "/abs/...", "version": "0.1.0" },
    { "coordinate": "agine.ai/api",        "root": "/abs/...", "version": "0.1.0" }
  ],
  "testServices": [
    { "coordinate": "agine.ai/api-tests", "subjectCoordinate": "agine.ai/api", "root": "/abs/...", "version": "0.1.0" }
  ]
}
```

- `bootstrap: true`（仅 `skiff.run/std`）：用 `skiff-package-service-smoke-fixture --bootstrap-only`
  播种，不走普通 publish。
- `root` 为绝对路径，由生成方解析（skiff → `SKIFF_ROOT`，skiff-packages → `SKIFF_PACKAGES_ROOT`，
  internals → internals 根）。`version` 从各 root 的 `package.yml` 读取。
- 引擎对 manifest 条目只做三件事：发布、stale 判定、base 服务集合推导。

## 4. Store 语义：全量与增量是一件事的两端

`--artifact-root <dir>` 指向的 store：

- **store 不存在或为空** → 全部 source stale → 全量发布（hermetic：从当前源码重建整个世界）。
- **store 已存在** → 与 store 内 sidecar 清单比对，只发布变化的 source（增量复用）。
- **`--fresh`** → 忽略 sidecar，全部强制发布。

也就是说没有"两个模式"，只有增量发布逻辑；store 新旧决定行为。用的人跑 `--plan` 即可在运行前
得知本次是全量还是增量。

### 4.1 Store sidecar 清单

`<store>/skiff-test-sources.json`（引擎写入、引擎读取）：

```json
{
  "createdAt": "2026-08-05T...Z",
  "sources": [
    {
      "coordinate": "agine.ai/llm-api",
      "kind": "package",
      "root": "/abs/...",
      "version": "0.1.0",
      "digest": "sha256:...",
      "files": [ { "path": "types.skiff", "sha256": "..." } ]
    }
  ]
}
```

- `digest` = sha256(按路径排序的 `path:sha256` 行串联)。
- 目录排除：`node_modules`、`.git`、`target`、`dist`、`build`、`.stack`、
  `.skiff-package-store`、`.vscode`。
- root 路径变化（换 checkout）不影响正确性：内容 hash 已覆盖；stale 判定只看 digest，
  发布后更新记录中的 root。

### 4.2 Stale 判定

| 情况 | 动作 |
| --- | --- |
| sidecar 缺失 / 条目缺失 | publish |
| `--fresh` | publish（全部） |
| digest 不同 | publish（计划中报告变更文件数） |
| digest 相同 | reuse（unchanged） |

### 4.3 发布方式

- `bootstrap: true`：`cargo run --manifest-path <skiff>/test-runner/Cargo.toml --bin skiff-package-service-smoke-fixture -- --bootstrap-only --artifact-root <store> --profile skiff-test --platform-source-root <skiffRoot>`。
- 其余：`skiff package publish <root> --artifact-root <store> --json`（包和服务同路径，服务 publish
  同时产出 service contract/deployment receipts）。
- 每发布一条即更新 sidecar；全部完成后落盘。

## 5. Base assembly 自动解析

现有 `skiff test` 要求 service 测试必须成对提供 `--base-assembly/--base-config-snapshot`（runner 从
base assembly 的 contracts 里匹配测试包的 service requirements，`test_service_fixture.rs`
`test_service_selectors`）。dev-home 下 "found 0" 是因为没有一份恰好含依赖服务合约的 assembly 记录。

新行为：提供 `--sources` 且未提供 `--base-assembly` 时，自动解析：

1. 测试 root 匹配 `manifest.testServices` 条目 → 得到 `subjectCoordinate`。
2. base services = `manifest.services` 排除 subject。
3. 逐个读取部署 pointer：`<store>/pointers/service-deployments/<coordinate（`.`→`~`）>/<version>.json`
   → `pointer.deployment`。缺失则报错（提示先完成发布 / `--fresh`）。
4. `skiff assembly build --artifact-root <store> --profile skiff-test --root-deployment <deployment JSON>... --json`
   → 得到 `runtimeAssemblyReceipt.assembly.assemblyIdentity` 与 recordPath（纯 JSON 投影，无编译）。
5. `cargo run --manifest-path <skiff>/config-snapshot-tooling/Cargo.toml -- --artifact-root <store>
   --assembly-record <recordPath> --profile skiff-test --source '<{"root":..., "deployment":...}>'...`
   → 得到 `runtimeConfigSnapshotReceipt.snapshot.snapshotId`。
6. 以成对身份传给 runner。

若用户显式提供了 `--base-assembly/--base-config-snapshot`，跳过解析直接用。

## 6. 分片与失败日志

- 发现：目录 root 递归收集 `*.test.skiff`（排除 `node_modules`、`.git`、隐藏文件）；文件 root 原样使用。
- 计数：`/^\s*test\s+"/gm`（与现有 sharded runner 一致）。
- 分片：按 case 数贪心分 `--shards` 份。
- 执行：每 shard spawn `node <skiff>/scripts/skiff.mjs test <files...> --artifact-root <store>
  [--base-assembly <id> --base-config-snapshot <id>] --deny-skips --require-tests`，并透传
  `CARGO_TARGET_DIR`、`SKIFF_TEST_MAX_CASES_PER_ACTIVATION`、`SKIFF_TEST_TRUSTED_SOURCE_ROOT`（若父进程
  设置了后者）。
- 失败：完整 stdout+stderr 写入 `<tmpdir>/skiff-sharded-test-logs/shard-<index>-<时间戳>.log`，
  打印文件路径，console 保留最后 25 行 tail。退出码非零。

## 7. `--plan` 输出（运行前可见）

```
mode:    hermetic full rebuild（store 不存在 / --fresh）   | incremental reuse（store 已存在，sidecar 匹配）
store:   /abs/path
sources:
  publish  agine.ai/api        （3 个文件变更）
  reuse    agine.ai/aihub      （unchanged）
tests:    agine/service-tests：9 个测试文件 / 42 个 case / 2 个 shard
base:     resolve from store：account、registry、codex-relay、aihub
```

`--plan` 不发布、不编译、不跑测试；正常运行时先打印同样内容再执行。

## 8. Internals 集成

1. 新增 `internals/scripts/write-source-manifest.mjs`：
   - 读 `canonicalSourceDefinitions` + `canonicalRootsFromEnvironment`（`SKIFF_ROOT` /
     `SKIFF_PACKAGES_ROOT`，缺失报错）。
   - 每个 root 读 `package.yml` 的 `version`。
   - 输出 §3 格式的 manifest，`--out <path>`（绝对路径）。
   - `skiff.run/std` 标记 `bootstrap: true`。
2. 删除 `internals/scripts/run-service-tests-sharded.mjs`。
3. `agine/service/package.json` 的 `test:service-tests:sharded` 改为：

   ```json
   "test:service-tests:sharded": "npm run guard:stable-mutation && node ../../scripts/write-source-manifest.mjs --out ../.skiff-test-sources.json && SKIFF_TEST_TRUSTED_SOURCE_ROOT=1 node ../../../skiff/scripts/skiff.mjs test ../service-tests --artifact-root ../.skiff-test-store --sources ../.skiff-test-sources.json --fresh --shards 8"
   ```

   `--fresh` 保证门禁 hermetic；开发循环去掉 `--fresh` 复用 `../.skiff-test-store`。
4. `internals/.gitignore` 追加 `.skiff-test-store/`、`.skiff-test-sources.json`。
5. 文档更新：
   - `skiff/test-runner/src/canonical_fixture.rs` 的 `SERVICE_TEST_FIXTURE_GUIDANCE` 指向新命令形态。
   - internals `AGENTS.md`、workspace `AGENTS.md`、`internals/scripts/AGENTS.md` 中提及
     `run-service-tests-sharded.mjs` / `skiff.mjs test` 的段落同步。
6. 兼容性：未删的 phase05 探针与 `prepare-canonical-assembly.mjs` 保留不动（canonical fixture 的
   门禁语义、source 清单、provenance 守卫仍由它们承载；本方案只把"测试执行编排"下沉 skiff）。

## 9. 错误与降级

- `--sources` 提供但测试 root 不在 `testServices` 且未提供 base 对 → 报错说明需要成对提供
  `--base-assembly/--base-config-snapshot`。
- store 中缺依赖服务部署记录 → 报错提示先完成发布（正常流程发布阶段会写入）或 `--fresh`。
- `--shards` 值非法（非正整数 / 0）→ 报错。
- `--plan` 遇到不可恢复错误（manifest 缺失 / store 不可读）→ 照常报错，exit 1。

## 10. 验证计划

1. skiff 单测（`scripts/tests/sharded-test.test.mjs`，`node --test`）：
   - `discoverTestFiles`（临时目录 fixture，排除规则）
   - `countTestCases` / `partitionTestFiles`（case 数均衡）
   - `sourceTreeDigest`（稳定性、变更检测、排除目录）
   - `planSourcePublish`（fresh → 全 publish；unchanged → reuse；变更 → publish + 文件数）
2. E2E（internals，agine 为例）：
   - `write-source-manifest.mjs --out ...` 生成 manifest，`skiff test ... --plan` 输出符合 §7。
   - `--fresh` 全量跑 `conversation_title_tool.test.skiff`（9 用例 PASS）。
   - 不改任何源码再跑：`mode: incremental reuse`，全部 reuse，耗时显著下降。
   - 改一个 service 文件后再 `--plan`：该条显示 publish（变更文件数）。
3. 门禁：agine `npm run test:service-tests:sharded` 全量通过（8 shard）。
4. 跨仓库提交：skiff（引擎 + 单测 + 引导文案）+ internals（generator / 删除 / npm / 文档）。
