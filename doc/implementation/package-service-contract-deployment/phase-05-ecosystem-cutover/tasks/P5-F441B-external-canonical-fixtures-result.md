# P5-F441B External manifest canonical fixture migration result

状态：`PASS / FIXTURE_MIGRATION_GREEN / EXPECTED_R0_RUNTIME_BLOCKED`。canonical fixture、
临时 fixture writer、copy receipt 与 current-generation golden 已完成 hard cut；两个 Runtime
selector 均在范围外 R0 exhaustiveness blocker 停止，没有越界修改 reader/execution production。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 production checkpoint | `67d61b8db9cb1750fe624dc40b9968642fb6d7f3` | `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff` |
| leaf dispatch HEAD | `a33b4810aefab9b1ad60f5aaddce3b07cb53487e` | `7de8adf011739af8b912803335e9232de518716d` |
| implementation | `6ed70177f407501b6d6298451ed20b11f3457fcc` | `be211535781f6d41ef2464a60a6a00c07f33a57f` |

dispatch HEAD 相对 production checkpoint 只增加 external-manifest migration leaf 调度文档。本
implementation 修改：

- `compiler/tests/fixtures/router-websocket-fixture/{service.yml,http.yml}`；
- 四个 `test-runner/fixtures/package-service-{i02-spawn-submit,websocket-generation-a,websocket-generation-b,websocket-smoke}/{service.yml,websocket.yml}`；
- `test-runner/tests/package_service_contract_deployment.rs`、
  `test-runner/src/package_service_host_fixture.rs`；
- 两个任务指定的 Runtime 临时 HTTP fixture writer；
- 三个任务指定的 script tests 及其直接 recursive-copy helper。

没有修改 parser/compiler production、Runtime/Router production、live root、cross-system checkpoint、
其它 task/result 或三仓真实 service。

## 2. Test-first RED 与迁移前 receipt

修改 checked-in fixture 前，先把两个 exact source fixture test 改为读取精简 `service.yml` 与独立
`websocket.yml`，再运行：

```bash
node --test scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs
```

结果为预期 `11 tests / 9 passed / 2 failed`；两个首错分别是缺少
`package-service-websocket-smoke/websocket.yml` 与
`package-service-i02-spawn-submit/websocket.yml`。随后才迁移文件。

迁移前 receipt 固定在 dispatch HEAD，并在改文件前保存了完整 fixture tree SHA-256 清单。五个被拆分
`service.yml` 的 exact SHA-256 为：

| Fixture | 迁移前 `service.yml` SHA-256 |
| --- | --- |
| `router-websocket-fixture` | `b03840e6b578d6395e6cded714796bf28deb4d3c76f5dde825326387623141ef` |
| `package-service-i02-spawn-submit` | `ff07b8cfe77903fd89614db8ec8bfd79a232fbbc5e7accb22f79705b40d1cc84` |
| `package-service-websocket-generation-a` | `d94276f9e123f0e294f2832dc4af46df05a161fb00f535ee44cd014ff862e50d` |
| `package-service-websocket-generation-b` | `d94276f9e123f0e294f2832dc4af46df05a161fb00f535ee44cd014ff862e50d` |
| `package-service-websocket-smoke` | `d94276f9e123f0e294f2832dc4af46df05a161fb00f535ee44cd014ff862e50d` |

## 3. Fixture 与 writer hard cut

- router compiler fixture 的 `service.yml` 只留 id；`ping` 原样移入 direct-map `http.yml`；
  `config.dev.yml timeout: 120000` 保持。
- 四个 checked-in WebSocket fixture 的 `service.yml` 只留 id/kind；`/socket + connect` 原样移入
  `websocket.yml`，没有伪造 JSON-RPC method。
- 其它 zero-ingress test-runner/std fixture 没有新增空 `http.yml`/`websocket.yml`。
- Runtime eval 与 Host 的临时 writer 都分别写最小 `service.yml` 和 direct-map `http.yml`；
  原有 profile timeout 仍只在 `config.dev.yml`。
- exact Node/Rust assertions 同时固定三文件 ownership；旧 inline 正例已清零，既有旧格式拒绝负例未改。

## 4. 真实 producer identity 前后矩阵

`ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures` 通过真实
`compile_package_project_for_test`、`generate_service_deployment` 与
`resolve_runtime_assembly` 生成 receipt；先采集 producer 输出，再把 exact assertion 固定为当前值，
没有手算或手调 hash。

PackageBuild v10 与 LocalAbi v7 在迁移前后 bit-identical：

| Fixture | PackageBuild（before = after） | LocalAbi（before = after） |
| --- | --- | --- |
| smoke | `skiff-package-build-v10:sha256:5ce089038445f6ea1bf05a5d8876ebb784c9193f4509ee993f0eb6b415c25880` | `skiff-package-local-abi-v7:sha256:d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba` |
| generation-a | `skiff-package-build-v10:sha256:25b98b03e66c0a7398859a6d0362dd53c20ff39f77ea36377408b74da6bfb37b` | `skiff-package-local-abi-v7:sha256:d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba` |
| generation-b | `skiff-package-build-v10:sha256:3f42eb72f997ccf6a65b986cb49af485ae63c67db32e5b587822cfadf9c5e791` | `skiff-package-local-abi-v7:sha256:d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba` |
| i02-spawn-submit | `skiff-package-build-v10:sha256:6f686ba330266ad08baf8dd04baba0bfcc315ec4e0ed8308344f9f8a7f8230b8` | `skiff-package-local-abi-v7:sha256:3db7056f815676834489b34a069b5016f05973b3be9379eb55736a545d7dcdf9` |

四组 connect GatewayEntry 共用同一 protocol surface：

- before：
  `skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936`
- after：
  `skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d`

DeploymentArtifact 与 RuntimeAssembly 的真实前后值：

| Fixture | DeploymentArtifact before → after | RuntimeAssembly before → after |
| --- | --- | --- |
| smoke | `skiff-deployment-artifact-v2:sha256:3e020a778a528ff61ddb4b953186299b9145beaa7c368bb8fa121a8c7db8ccf5` → `skiff-deployment-artifact-v3:sha256:787c89e6ca10c1b3d29fd30ff4f6fae9791113d6a98ae5f96401940e546a71fb` | `skiff-runtime-assembly-v2:sha256:d949679862b2e0b5cff67cbd517bab56b6bb7b2165906a3860811b3db181c342` → `skiff-runtime-assembly-v2:sha256:80d53e6b18e987a61959557b0343835b5f126366e98e27510dcc8ae0e86ec664` |
| generation-a | `skiff-deployment-artifact-v2:sha256:6a4e17954474836b8a2511442e44855b16a0d51d77e4b82fd90d8842daf9c9c5` → `skiff-deployment-artifact-v3:sha256:bdc58ca03bb32567156582d4725e67b2efca9a8398586901360d44e0ac52fd21` | `skiff-runtime-assembly-v2:sha256:6cae8bf053cba5247aafe4ef4ab635d453cd9688f6935ad01891679d6ed3f1dd` → `skiff-runtime-assembly-v2:sha256:135a4afee48fa8c7a729ecd317620e5c4615ea08949f926aed9b7b3239becca9` |
| generation-b | `skiff-deployment-artifact-v2:sha256:0897f23f6972709688cf420e30589ff2cb64380cd63a4e45cc6938aa96308d8b` → `skiff-deployment-artifact-v3:sha256:e9d5508571436a9b6e1516a7f662cf76b16c328422adb9b0df602fc443bfccf2` | `skiff-runtime-assembly-v2:sha256:f73298e2908b53e535d1fe4f6b7c654166a304c1f0468b0c88a8feba08c4f079` → `skiff-runtime-assembly-v2:sha256:5dd9eb3300f0bb95e9ceca0a59a3080f2879dbf4816f57643969bc46ab6e55ec` |
| i02-spawn-submit | `skiff-deployment-artifact-v2:sha256:6eb0fffd40ee1d373db397063ae81e587ec564852be149bfa1f225bc763c8766` → `skiff-deployment-artifact-v3:sha256:4bfe86f13a9a5622b1c323601624585e7132cb22cc6caf0b0027b73e79b3efa8` | `skiff-runtime-assembly-v2:sha256:11b8f9d38d44c642438f37a6b787b58b3deca8545d9c48d850a6a0c00813752a` → `skiff-runtime-assembly-v2:sha256:b479a7b5dc4e1cb966448039dd93af20875b9c1e041e92b675cc6511869cad37` |

HTTP run/probe surface没有因 manifest 搬家改变；其 exact golden 只消费 M1 已冻结的 current gateway v2
marker/preimage，并由真实 fixture producer刷新：

| Entry | before → after |
| --- | --- |
| package-test `run` | `skiff-gateway-entry-v1:sha256:cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4` → `skiff-gateway-entry-v2:sha256:b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305` |
| smoke `probe` | `skiff-gateway-entry-v1:sha256:adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653` → `skiff-gateway-entry-v2:sha256:94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705` |

## 5. Recursive-copy receipt

- Node negative Host probe 继续使用 `cp(..., { recursive: true })`；directory snapshot先证明完整文件集只改
  `main.test.skiff`，新增 receipt 再逐个证明 external control file 的 path/size/SHA-256 不变。
  focused temporary vector 的 `http.yml` receipt 为
  `144 bytes / a448ff082babd8ebb2fbca99b7a04d20a15c73d6ac74fcc1d82dd49e38301309`。
- `test-runner` 两个 recursive Rust helper 都增加 nested `http.yml`/`websocket.yml` exact-byte
  copy receipt test；实现仍递归复制整棵 tree，没有改成文件白名单。
- checked-in zero-ingress Host fixture 本身没有新增空 external 文件。

## 6. 验证结果

所有 Cargo 命令均使用
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures` | PASS，1 passed |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment websocket` | PASS，1 passed |
| 三个任务指定的 `node --test` 文件 | PASS，15 passed |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment http_fixture` | PASS，2 passed |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment i02_submit_probe_is_private_http_gateway_not_service_operation` | PASS，1 passed |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment recursive_copy_tree_receipt_preserves_external_control_files` | PASS，1 passed |
| `RUSTFLAGS='--cfg p5_f441b_force_rebuild' cargo test -p skiff-test-runner --lib recursive_copy_fixture_tree_receipt_preserves_external_control_files`（仍用同一 shared target） | PASS，1 passed |
| `cargo fmt --all -- --check` | PASS |
| 四个变更 Node 模块的 `node --check` | PASS |
| `git diff --check` | PASS |

规定反向搜索

```bash
rg -n '^[[:space:]]*(http|websocket|timeout):' --glob service.yml \
  compiler/tests/fixtures test-runner test-services
```

为 0 命中。`test-runner/tests/package_service_contract_deployment.rs` 中旧
`skiff-gateway-entry-v1` / `skiff-deployment-artifact-v2` positive golden 也为 0 命中。

## 7. 精确范围外 blocker

以下两个任务规定命令都未进入本 leaf 的临时 writer test body：

```bash
cargo test -p skiff-runtime-eval runtime_http_gateway
cargo test -p skiff-runtime-host runtime_assembly_request
```

两者的共同首错是：

```text
runtime/eval/src/runtime_http_gateway.rs:85
E0004: GatewayAdapterKind::WebSocketJsonRpc not covered
```

同一 R0 exhaustiveness blocker 还包括：

- `runtime/eval/src/runtime_http_gateway.rs:439` 未消费
  `WebSocketJsonRpcParams` / `WebSocketBusinessIdentity`；
- `runtime/eval/src/runtime_websocket_connect.rs:171` 未消费同两种 source。

这些文件属于 R0 Runtime execution/readers，不在 F441B 写集；本 leaf 没有增加 wildcard、compatibility
arm 或越界修复。未运行 live、instance、watch、stable、固定端口 workload；未 merge、rebase 或 push，
未派子 agent。
