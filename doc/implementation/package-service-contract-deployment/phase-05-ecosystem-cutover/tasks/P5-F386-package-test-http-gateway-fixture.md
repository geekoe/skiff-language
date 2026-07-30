# P5-F386 Package-test HTTP gateway fixture

状态：Ready（F384 T1；依赖F385）。

## 直接父节点

- `P5-F384-test-assembly-gateway-control-plane-audit-result.md`
- `P5-F385-router-test-gateway-control-plane-result.md`

只实现父审计§4.1、§4.2、§5、§6 T1与§7.2冻结的package-test consumer。Ecosystem smoke是后继T2，
本节点不改。

## Worktree

- `/Users/geek/workspace/skiff-p5-f386-package-test-http-gateway`
- branch `codex/p5-f386-package-test-http-gateway`
- base：包含本任务与F385的Skiff phase-05 integration。

## Canonical fixture

1. 新增test-runner-owned小型canonical gateway helper：
   - exact private `InternalFunction` handler；
   - `typedJson` unary；
   - `body <- http.body`；
   - request schema Null，response schema Null；
   - `pre/guard = null`，fixed external error projection v1；
   - identity必须调用现有`gateway_entry_identity`，不得手拼digest。
2. 每个test case生成private
   `<test>Gateway(body: null) -> null` wrapper，调用现有零参test body后返回null；wrapper不得进入API。
3. 每case artifact/assembly：
   - zero-operation contract，empty package type requirements；
   - empty operation bindings；
   - exactly one gateway entry/ingress；
   - key `run`；
   - selector host `case-{index}.package-test.skiff.localhost`、POST、
     `/__skiff/package-test/{index}`；
   - canonical gateway identity
     `skiff-gateway-entry-v1:sha256:cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4`。
4. 保持base assembly service/package bindings与per-case state isolation。

## Runtime control

按父审计§5 exact发送F385 strict control object：

- 不再发送contract/deployment/key、`testEffectsEnabled`或`testEffectDoubles`；
- routing/gateway/mode/selector取自刚发布并激活的canonical facts；
- payload exact canonical Base64 `null`；
- strict decode runtime `response.end`；
- outer 2xx不等于case成功，必须验证inner HTTP 200、content type与payload exact `null`；
- error/wrong type/invalid Base64/unknown field/payload flag不一致均fail closed。

inline setup保持test doubles唯一来源；wire为空。测试路径经F385私有seam产生
`testEffectsEnabled=true`，production路径不变。

## 写入边界

允许：

- 新test-runner-only canonical gateway helper；
- `test-runner/src/test_overlay.rs`
- `test-runner/src/package_test_assembly.rs`
- `test-runner/src/runtime_execution.rs`
- `test-runner/src/runtime_execution/wire.rs`
- `test-runner/src/runtime_execution/tests/**`
- `test-runner/tests/package_service_contract_deployment.rs`

禁止：

- ecosystem smoke fixture/scripts；
- `runtime/package-test/src`；
- Router、Host/eval/transport、WebSocket；
- skiff-packages/Internals/stable/live。

## 验收

正负矩阵按父审计§7.2。至少运行：

```bash
cargo test --locked -p skiff-test-runner --lib runtime_execution -- --test-threads=1
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1
cargo check --locked -p skiff-test-runner --bins
cargo clippy --locked -p skiff-test-runner --all-targets --no-deps -- -D warnings
git diff --check
```

scoped production反搜旧contract operation/doubles字段为零。必须至少运行一个真实isolated package-test，
证明fixture assembly、Router control、Host/eval和inline setup贯通；不得只过结构测试。

写`P5-F386-package-test-http-gateway-fixture-result.md`，production/tests/result本地commit，worktree
clean；不merge/rebase/push。新Agent执行，不派子Agent。若需要改F359/F365/Router或WS，返回
`TASK_SCOPE_EXPANDED`。
