# P5-F441B External manifest canonical fixture migration

状态：Ready。对应 F440A 冻结 DAG 的 S1；确定性独立 leaf。

## 直接父节点

- `P5-F440A-external-manifest-owner-audit-result.md`
- `P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`
- `P5-F440M-external-manifest-identity-deployment-follower-result.md`

实现基线为 `67d61b8db9cb1750fe624dc40b9968642fb6d7f3`
（tree `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff`）。

## 目标

把 Skiff canonical test fixtures、临时fixture writer和对应golden切到split manifest：

- `service.yml`只保留 `id`、可选`kind`、可选`serviceCalls`；
- HTTP写入`http.yml`，WebSocket写入`websocket.yml`；
- timeout只保留既有`config.<profile>.yml` owner；
- recursive-copy helper继续复制完整tree，并由receipt证明external文件未丢；
- Package build与Local ABI的既有golden保持原值；
- Gateway v2、DeploymentArtifact v3与由其自然变化的assembly值必须由真实producer重算，禁止手调hash。

本 leaf只迁已有connect-only fixtures，不实现JSON-RPC broker/runtime。

## 唯一写集

- `compiler/tests/fixtures/router-websocket-fixture/**`
- `test-runner/fixtures/**`
- `test-services/std/**`
- `test-runner/tests/package_service_contract_deployment.rs`
- `test-runner/src/package_service_host_fixture.rs`
- `runtime/eval/src/runtime_http_gateway/tests.rs`中的临时service fixture writer
- `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs`
- `scripts/tests/package-service-i02-combined.test.mjs`
- `scripts/tests/package-service-ecosystem-http-fixture.test.mjs`
- `scripts/tests/package-service-host-negative-probe.test.mjs`
- 上述测试使用的直接recursive-copy helper
- 本 leaf result

禁止修改 parser/compiler production、Runtime/Router production、live roots、cross-system checkpoint、
其它 task/result或三仓真实service。不得派子 agent。

## 迁移不变量

至少迁移：

- `router-websocket-fixture`：`http.yml`中保留`ping`，service只留id，dev timeout不变；
- `package-service-i02-spawn-submit`；
- `package-service-websocket-generation-a`；
- `package-service-websocket-generation-b`；
- `package-service-websocket-smoke`；
- 两个runtime临时HTTP service writer。

无ingress fixtures不得伪造空external文件。旧 inline `http/websocket/timeout`正例清零；旧格式拒绝负例保留为
负例，不得通过兼容读取让它们转绿。

四组 checked-in WebSocket fixture：

- PackageBuild v10和LocalAbi v7 exact值保持；
- connect GatewayEntryIdentity刷新为gateway v2真实值；
- DeploymentArtifact刷新为v3真实值；
- RuntimeAssembly generation不升代，但identity按新deployment ref真实刷新。

HTTP run/probe gateway identity只在真实preimage变化时刷新，不能因manifest搬家而改。

## 测试先行与验证

先让exact fixture断言在旧inline文件上失败，再迁移；保存改前identity receipt并在result列出前后矩阵。

必跑：

```bash
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures
cargo test -p skiff-test-runner --test package_service_contract_deployment websocket
node --test scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs \
  scripts/tests/package-service-host-negative-probe.test.mjs
cargo test -p skiff-runtime-eval runtime_http_gateway
cargo test -p skiff-runtime-host runtime_assembly_request
cargo fmt --all -- --check
git diff --check
```

Cargo命令统一使用共享
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。

反向搜索：

```bash
rg -n '^[[:space:]]*(http|websocket|timeout):' --glob service.yml \
  compiler/tests/fixtures test-runner test-services
```

若运行时loader因尚未合流的R0 reader阻塞，只能记录精确首错；fixture生成、compiler/test-runner可独立验证的
部分仍须完成。不得为通过测试越界修改reader。

## 停止与交付

需要修改 write set外的copy owner、parser或runtime production时返回`TASK_SCOPE_EXPANDED`。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441b-external-canonical-fixtures`
- branch：`codex/p5-f441b-external-canonical-fixtures`
- result：`P5-F441B-external-canonical-fixtures-result.md`

Implementation 与 result 分开提交；不 merge/rebase/push。
