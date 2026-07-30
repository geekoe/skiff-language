# P5-F441I Canonical live source-root authoring

状态：Ready。F441C 的 authoring 后继；只迁 source roots，不运行或改造 live harness。

## 直接父节点

- `P5-F441C-external-live-root-migration-result.md`
- `P5-F441H-test-service-profile-target-environment-separation-result.md`
- `P5-F441A-external-control-file-discovery-result.md`
- `P5-F415-collection-mapping-current-integration-result.md`
- `P5-F388-legacy-live-service-authoring-audit-result.md`

实现基线为 `c3878e3df9e010381bc6bf0dcfb60379e5f6dcf7`
（tree `5256045bdc82c89eac7c878b3cbb901cf8130fb1`）。

F388 只作为三个 root、两个 dependency package、40 条 HTTP entry、private wrapper、DB/file normal-source
owner与collection mapping的代码事实父节点。其早于当前 `kind: test` 模型的
`config.dev.yml` / base-assembly /旧 gateway identity结论已被 testing reference、F440M和F441H取代，
不得恢复。

## 目标

一次完成三个 tracked source root 的 canonical authoring：

- `runtime/encrypted-storage-live/default-service`
- `runtime/encrypted-storage-live/mapped-service`
- `runtime/live-tests`

终态共 40 个 HTTP ingress：default 21、mapped 13、runtime-live 6；39 unary、1 server-stream。
全部是 `rawHttp`，每条唯一参数均为 `request <- http.request`。`typedJsonEcho` 仍是内部手工 decode 的
raw handler；`runtime.guarded` 仍是 GET selector；不得“修正”业务行为。

## 唯一写集

- `runtime/encrypted-storage-live/default-service/**`
- `runtime/encrypted-storage-live/mapped-service/**`
- `runtime/encrypted-storage-live/package-store/**`
- `runtime/live-tests/**`
- `test-runner/tests/package_service_contract_deployment.rs` 中只允许新增本 leaf 的真实 root compile /
  receipt 测试及其直接 helper
- 本 leaf result

禁止修改 test-runner production、scripts/harness/plan、Compiler、Router/Runtime production、其它
fixture、其它 task/result、stable/live 状态。不得派子 agent。

## Canonical control files

三个 service root 都必须拥有：

- `package.yml`
- `api.yml`
- 精简 `service.yml`
- `http.yml`
- 一个固定 profile 文件

精确角色：

1. encrypted default / mapped 是 ordinary service：
   - `service.yml`只保留 `id`；
   - profile 是 `config.dev.yml`；
   - `timeout: 120000`，并为真实 config/state requirements 提供 normal deployment binding；
   - `package.yml`拥有 version、state与 mapped package dependency/mapping。
2. `runtime/live-tests` 是 test service：
   - `service.yml`为 `id: skiff.run/runtime-live`、`kind: test`；
   - profile固定为 `config.skiff-test.yml`，不得新增或读取 `config.dev.yml`；
   - live target environment不进入文件名或profile selection；
   - `package.yml`拥有 version、state与runtime-kit package dependency。
3. 三个 `api.yml` 都是 `{}`，三个 ServiceContract 都是 zero-operation。
4. 不创建空 `websocket.yml`，不保留 legacy inline `http`、`routes`、`timeout`、`version`或`packages`。

## HTTP 与 source 迁移

- 把 40 条 legacy route 按 F388 第 2 节矩阵迁入顶层 entry map `http.yml`；
- key 使用冻结的 `default.*`、`mapped.*`、`runtime.*`；
- handler/guard 删除废弃 `root.`，全部绑定 current implementation package；
- default/mapped 的 34 条 entry逐条复制 `guard: internal.live.guard`；
- 每条 entry显式写 `kind: rawHttp`和唯一 `adapterArgs`；
- runtime package route必须由
  `runtime/live-tests/internal/http_adapter.skiff::packageEcho(HttpRequest) -> HttpResponse`
  private wrapper调用 `runtimeKit.packageEcho`，gateway不得直接绑定 dependency alias；
- runtime kit移入 source collector忽略的 `.skiff-packages` canonical package目录，拥有独立
  `package.yml`与`api.yml`；删除旧 inline API owner和会被递归收集的旧位置；
- encrypted store dependency保留独立 canonical package，并使用当前 accepted
  `collection_name_mapping`；F415 已拥有 mapping transport，不得兼容旧 camelCase 字段。

## Test-service production ownership

test-only declaration 不进入 production PackageArtifact，因此 runtime-live 的真实 deployment requirement
必须由 normal private source拥有：

- `RuntimeLiveDoc`、DB object和 DB probe helper移入 `internal/db_live.skiff`；
- file stream/helper移入 `internal/file_live.skiff`；
- operation、DB、file、HTTP marker的 config requirement由 normal private source accessor/handler拥有，
  test-only 文件只调用这些 owner并保留 assertions；
- `package.yml`声明 `runtime-live-store`，`config.skiff-test.yml`精确绑定它；
- 删除退出使用的 `runtime-live.config.example.json`，不得实现 per-case config override。

default ordinary root中测试使用的 `encryptedLive.testRunnerSecret`也必须由 normal private accessor声明，
`config.dev.yml`才能合法绑定固定测试值；不得恢复 runner `--config` 注入。

本 leaf 不解决 `__skiffPayload` 自定义输入或 over-limit expected-platform-error执行语义；这两项留给
后续 test execution owner，但 source root必须能 canonical compile。

## 测试先行与验收

先在允许的 test-runner integration 文件新增真实 root probe，使 legacy roots因缺
`package.yml`、inline control或 source ownership至少一项失败，再迁移。

终态测试必须从 fresh temporary artifact store：

1. 发布两个 dependency package；
2. 编译 default、mapped ordinary service和runtime-live test service；
3. 读取真实 producer records并断言：
   - package/service coordinate正确；
   - 三个 contract均为 0 operation；
   - deployment gateway/ingress分别 `21/21`、`13/13`、`6/6`；
   - 合计 40 entry、39 unary、1 server-stream；
   - 每个 selector/key/handler/guard/adapter arg与 F388 current matrix一致；
   - gateway identity使用 current v2真实producer值，禁止复制 F388 的 v1 golden；
   - mapped dependency edge携带 `package_secret -> mapped_package_secret`；
   - runtime-live实际选择 `config.skiff-test.yml`。

必跑：

```bash
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  canonical_live_source_roots
cargo test -p skiff-test-runner --test package_service_contract_deployment
cargo fmt --all -- --check
git diff --check
```

Cargo 命令统一使用：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

反向搜索：

```bash
rg -n '^[[:space:]]*(version|packages|http|routes|timeout):|root\\.' \
  --glob service.yml --glob http.yml \
  runtime/encrypted-storage-live/default-service \
  runtime/encrypted-storage-live/mapped-service runtime/live-tests
rg -n 'collectionNameMapping|runtime-live\\.config\\.example|package-store/.+runtime-live-kit' \
  runtime/encrypted-storage-live runtime/live-tests
```

第一条允许 `http.yml` entry内部正常的 `source: { kind: http.request }`，不允许 legacy owner/key；
result需分类而非机械宣称零匹配。

## 停止与交付

若 canonical compile需要修改 parser/compiler/runtime/test-runner production或 scripts，返回
`TASK_SCOPE_EXPANDED`并给出精确 blocker；不得越界。禁止启动 Mongo、Router、Runtime、telemetry、
instance、watch、stable或任何 live workload。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441i-live-source-authoring`
- branch：`codex/p5-f441i-live-source-authoring`
- result：`P5-F441I-canonical-live-source-root-authoring-result.md`

Implementation 与 result 分开提交；不 merge/rebase/push。
