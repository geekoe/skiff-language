# P5-F441A External control-file discovery result

状态：`PASS / S0_IMPLEMENTATION_GREEN / EXPECTED_S1_GATE_BLOCKED`。本 leaf 已完成
`http.yml` / `websocket.yml` 的 classifier、watch fingerprint、publication resource/archive 与
test-runner role discovery hard cut。任务规定的精确 test-runner package 命令仍在既有 S1 fixture
owner 编译边界停止；本 leaf 的 `--lib canonical_package` 动态证据全部通过，未越界修改 S1。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| production checkpoint | `67d61b8db9cb1750fe624dc40b9968642fb6d7f3` | `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff` |
| leaf dispatch | `a33b4810aefab9b1ad60f5aaddce3b07cb53487e` | `7de8adf011739af8b912803335e9232de518716d` |
| implementation | `ff89ac23274b6d084e2ff696d549c8c528855db6` | `939d4958805756e0861f7185a182ea2a18012cdc` |

Implementation 只修改任务授权的 9 个文件：

- `scripts/{skiff.mjs,skiff-dev-sync.mjs,check-publication-resource-archive.mjs}`
- `scripts/lib/publication-resources.mjs`
- `scripts/tests/{package-service-dev-sync.test.mjs,skiff-test-cli.test.mjs}`
- `compiler/input/src/resources.rs`
- `test-runner/src/canonical_package.rs`
- `test-runner/src/canonical_package/tests.rs`

本文是独立 result-only 提交；其 commit/tree 由最终交付消息记录。

未修改 M0 reader、checked-in service fixture、package archive producer、compiler
authoring/projection、artifact identity、deployment、Runtime、Router或三仓真实 service root。

## 2. Test-first red

Production 修改前得到以下真实失败：

1. Node focused suite：
   - `package-service-dev-sync.test.mjs` 因尚无
     `watchAuthoringRootChanges` export 在 module instantiate 阶段失败；
   - `skiff-test-cli.test.mjs` 的 external-only case 实际返回
     `must contain package.yml`，未命中 external/service invariant。
2. `cargo test -p skiff-compiler-input resources`：
   `7 passed / 1 failed`；新增 `http.yml` control-file vector 被旧 denylist 接受。
3. `cargo test -p skiff-test-runner --lib split_external_manifests_require_and_preserve_the_service_role_marker`：
   `0 passed / 1 failed`；external-only root 被错误返回为 `None`。
4. `node scripts/check-publication-resource-archive.mjs`：
   `resources: ["http.yml"]` 未产生预期 `control file` 拒绝。

这些 red 都由本 leaf 的 production 变更转绿，没有兼容 alias、dual-read或第二套 watch 清单。

## 3. Root classification 与 role discovery

- `detectRootKind` 和 `classifyAuthoringRoot` 都把 `http.yml` / `websocket.yml` 识别为 external
  service control file。
- ordinary package + external、external-only 均在 Cargo/compiler 之前 terminal fail closed；
  error 明确要求同 root 的 `service.yml` 声明 service role。
- ordinary package、service-only、manifest-less 与 retired `contract.yml/deployment.yml` 保留原语义。
- package + service + 任意一个或两个 external 文件仍分类为 package；external 文件本身不创造
  service role。
- test-runner 仍先用 regular `service.yml` 判断是否存在 service role；存在 role 后只调用 M0
  `read_service_package_root` 读取 typed split root。
- 若没有 `service.yml` 但存在任一 external control path，test-runner 也通过同一个 typed root
  reader产生 terminal error，不把 external 文件当 test service marker。合法
  `kind: test + http.yml + websocket.yml` profile 正常读取，非法 external DTO由 typed reader拒绝。

## 4. Watch、resource 与 archive 边界

- watcher 继续只使用现有 `rootsFingerprint/hashTree` 全树 path+bytes hash；没有新增 external
  filename watch list。
- 主 watch loop 与 direct test 现在共用 `watchAuthoringRootChanges`。测试依次修改
  `http.yml` bytes、删除该文件、新增 `websocket.yml`，每次 change 后再做一次 unchanged poll，
  精确得到 `3 changes / 3 rebuild callbacks`。
- Rust `is_skiff_control_file` 与 JS publication control-file denylist 均加入两个 external 文件；
  package resource declaration无法把它们吸收进 `PackageArtifact`。
- package source archive producer保持不变；checker 在 source root 实际放入两个 external 文件，
  exact archive仍只有 `package.yml`、declared ordinary resource与 `.skiff` source。
- `visitManifestDirectories` production 逻辑保持只访问 `package.yml` / `service.yml`。direct checker
  即使把 external filenames 传入 candidate set，也证明 `http.yml` / `websocket.yml` 中伪造的
  `resources` 不会被当作 resource manifest。

## 5. 验证结果

所有 Cargo 命令均使用
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。

| 命令 | 结果 |
| --- | --- |
| `node --test scripts/tests/package-service-dev-sync.test.mjs scripts/tests/skiff-test-cli.test.mjs` | PASS，`16 passed / 0 failed` |
| `cargo test -p skiff-compiler-input resources` | PASS，`8 passed / 0 failed` |
| `cargo test -p skiff-test-runner --lib canonical_package` | PASS，`2 passed / 0 failed / 2 ignored` |
| `node scripts/check-publication-resource-archive.mjs` | PASS |
| `node --check scripts/skiff.mjs` | PASS |
| `node --check scripts/skiff-dev-sync.mjs` | PASS |
| `node --check scripts/lib/publication-resources.mjs` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

实际执行的动态 test selectors 合计 `26 passed / 0 failed`；test-runner 的两个既有 merge-only
probe保持 ignored。

任务原列的精确命令

```bash
cargo test -p skiff-test-runner canonical_package
```

在本 leaf unit test 执行前被父结果已归属 S1 的同一 integration test 阻断：

```text
error[E0609]: no field `websocket` on type `ServiceManifestAuthoring`
  --> test-runner/tests/package_service_contract_deployment.rs:2257:10

error[E0063]: missing fields `http` and `websocket` in initializer of
`GeneratedServiceDeploymentInput<'_>`
  --> test-runner/tests/package_service_contract_deployment.rs:2307:50
```

`test-runner/tests/package_service_contract_deployment.rs` 属于 F441B/S1 checked-in fixture
写集，不属于本 leaf；按唯一写集未修改。`--lib canonical_package` 已证明本 leaf direct role
discovery tests转绿。

## 6. Reverse search 与隔离

规定的

```bash
rg -n 'service\.yml|package\.yml|http\.yml|websocket\.yml' \
  scripts/skiff.mjs scripts/skiff-dev-sync.mjs scripts/lib \
  compiler/input/src/resources.rs test-runner/src/canonical_package.rs
```

逐项分类结果：

- `skiff.mjs` 命中全部属于 source-root role classifier；
- `skiff-dev-sync.mjs` 命中全部属于同一 authoring-root inventory；fingerprint仍按全树运行；
- `publication-resources.mjs` 的 manifest walk仍只有 package/service，external只出现在
  control-file denylist；
- `package-source-archive.mjs` 仍只显式拥有 package manifest、declared resources与 `.skiff`；
- Rust resources命中为完整 control-file denylist及 direct vectors；
- test-runner production只通过 exported filename constants消费 split control paths，没有复制
  external DTO parser。

未运行 workspace 全测、instance、stable、live、watch registry、reload或固定端口 workload；
未 merge、rebase、push，未派子 agent。
