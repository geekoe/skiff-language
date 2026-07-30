# P5-F441A External control-file discovery and archive hard cut

状态：Ready。对应 F440A 冻结 DAG 的 S0；确定性独立 leaf。

## 直接父节点

- `P5-F440A-external-manifest-owner-audit-result.md`
- `P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`

需要细节时只沿这些父节点引用向上读取。实现基线：

- Skiff production checkpoint：`67d61b8db9cb1750fe624dc40b9968642fb6d7f3`
- tree：`6ffd7924e0e7359e3ffd2f05635bd724a2d961ff`

## 目标

让所有 root classifier、dev-sync、resource/archive validator 与 test-runner role discovery 正确消费
独立 `http.yml` / `websocket.yml`：

1. external 文件是 service control file，不是 package source/resource；
2. ordinary package 或 manifest-less root 中出现 external 文件必须 terminal fail closed；
3. 合法 service root 仍由 `service.yml` 声明角色，external 文件不能单独创造 service；
4. dev-sync 对 external 文件内容变化产生一次 rebuild，删除/新增也进入 fingerprint；
5. package source archive、PackageArtifact 与 resource archive都不得吸收 external 文件；
6. test service profile discovery继续以 `service.yml`判断角色，再由 typed root reader读取split文件。

不改变 M0/M1 DTO、compiler projection、identity generation或checked-in service fixture。

## 唯一写集

- `scripts/skiff.mjs`
- `scripts/skiff-dev-sync.mjs`
- `scripts/lib/publication-resources.mjs`
- `scripts/check-publication-resource-archive.mjs`
- 上述模块的直接 CLI/dev-sync/resource/archive tests，重点是：
  - `scripts/tests/package-service-dev-sync.test.mjs`
  - `scripts/tests/skiff-test-cli.test.mjs`
  - publication/resource/archive checker直接 tests
- `compiler/input/src/resources.rs`及其 colocated/direct tests
- `test-runner/src/canonical_package.rs`及其 colocated/direct tests
- 本 leaf result

禁止修改 checked-in service fixtures、compiler authoring/projection、artifact identity、deployment、
Runtime、Router、其它 task/result或三仓真实 service root。不得派子 agent。

## 必须固定的行为

- `http.yml` / `websocket.yml` 进入 `is_skiff_control_file` 与所有等价 control-file denylist。
- `visitManifestDirectories`仍只遍历可以声明resources的manifest；不得把 external DTO 当resource
  manifest，但要由测试固定此窄职责。
- package source archive不复制 external 文件；把它们声明成resource时必须拒绝。
- `detectRootKind` / `classifyAuthoringRoot`覆盖：
  - service + optional external：合法；
  - package + external、无service：拒绝；
  - external-only：拒绝；
  - manifest-less：保持现有拒绝；
  - service-only与ordinary package：保持现有合法语义。
- watcher改 `http.yml`或`websocket.yml`的bytes、新增或删除文件，均进入root fingerprint并恰好触发
  一次build；不得新增第二套watch清单。
- 不把只读取service id/kind的窄consumer误改成完整parser。

## 测试先行与验证

先增加能在候选基线失败的 classifier/resource/archive/watcher用例，再实现。

必跑：

```bash
node --test scripts/tests/package-service-dev-sync.test.mjs scripts/tests/skiff-test-cli.test.mjs
cargo test -p skiff-compiler-input resources
cargo test -p skiff-test-runner canonical_package
node scripts/check-publication-resource-archive.mjs
node --check scripts/skiff.mjs
node --check scripts/skiff-dev-sync.mjs
node --check scripts/lib/publication-resources.mjs
cargo fmt --all -- --check
git diff --check
```

逐项分类：

```bash
rg -n 'service\\.yml|package\\.yml|http\\.yml|websocket\\.yml' \
  scripts/skiff.mjs scripts/skiff-dev-sync.mjs scripts/lib \
  compiler/input/src/resources.rs test-runner/src/canonical_package.rs
```

Cargo命令统一使用
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`，不得创建第二套大
target。不得运行workspace全测、live、instance或stable。

## 停止与交付

若必须修改 M0 reader、checked-in fixture或package archive生产器本身以外的新owner，返回
`TASK_SCOPE_EXPANDED`并精确列文件/首错；不得扩写集。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441a-external-control-discovery`
- branch：`codex/p5-f441a-external-control-discovery`
- result：`P5-F441A-external-control-file-discovery-result.md`

Implementation 与 result 分开提交；不 merge/rebase/push。
