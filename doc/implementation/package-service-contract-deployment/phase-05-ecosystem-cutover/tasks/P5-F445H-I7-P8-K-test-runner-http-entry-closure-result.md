# P5-F445H I7 P8 K Test-runner HTTP entry closure result

状态：

```text
PASS
TASK_SCOPE_EXPANDED = NO
READY_FOR_INTEGRATION = YES
```

## 1. Baseline与预检

- baseline commit：
  `45a89dc40dd2f4cffc19296acc9a31065fcc3a37`
- baseline tree：
  `e67bfc6553b9a59797b04a4722768ee765529947`
- 预检分类：
  `EXISTING_CAPABILITIES_COMPOSITION`

零worktree预检确认：

- runner已经持有动态business ingress URL；
- 每个case已经有唯一synthetic service id、contract version、deployment与activation generation；
- 父`test-dispatch` frame已经携带exact deployment/generation和普通`httpRequest.url`；
- compiler已有普通`http.yml` gateway projection producer；
- 缺口只在test-runner没有保留test service的显式HTTP authoring、没有把它投影进case deployment、
  没有注入runner ingress config，并仍给父frame写`http://localhost`占位URL。

因此未修改compiler、std、File IR、Router或Runtime，也未新增URL scheme、header、session、token、
artifact或wire字段。

## 2. RED

baseline上的聚焦RED：

1. 显式test-service `http.yml`完全未进入case deployment；带
   `config.require<string>("skiff.test.ingressUrl")`的fixture先失败为
   `missing config binding skiff.test.ingressUrl`。
2. `lifecycle.maxConcurrency: 1`的HTTP-entry test service仍能成功assembly，证明缺少同deployment
   父control与self HTTP所需的并发门禁。
3. 父control frame的`httpRequest.url`仍是
   `http://localhost/__skiff/package-test/0`，不是runner提供的动态business ingress。

这些RED分别对应本任务的deployment/config、并发安全和execution-context输入改动。

## 3. Implementation

- `CanonicalTestServiceProfile`保留typed `http.yml` authoring；普通subject package/service ingress不被
  自动复制。
- runner复用compiler现有`generate_service_deployment`只投影显式HTTP gateway/ingress，再与现有
  package-test control gateway合入同一个普通case deployment。
- 每个case继续使用既有synthetic service id和package contract version；两个case的service id负例/正例
  已覆盖。
- 仅当编译后的test service真实要求`skiff.test.ingressUrl`时，runner把规范化后的动态ingress origin作为
  普通config literal注入；authored config或secret同名binding均fail closed。
- `http.yml`含entry时要求`lifecycle.maxConcurrency >= 2`；没有拆分control/target deployment。
- control请求仍发送到既有control endpoint；其现有frame内`httpRequest.url`改用真实business ingress
  origin。exact deployment/generation仍由原`routing`字段提供，供H沿现有test execution context消费。
- library path现在也在任何activation网络请求前校验ingress URL；缺失和非origin URL均fail closed。
- 新增HTTP entry/profile两个小owner文件，避免继续扩大原本超过数百行的assembly文件。

## 4. Evidence

最终聚焦gate：

```text
cargo check --locked -p skiff-test-runner --tests
PASS

cargo test --locked -p skiff-test-runner --no-fail-fast
76 passed, 0 failed, 3 ignored

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

关键覆盖：

- 两个case得到不同synthetic service id；
- 每个case deployment同时含一个control ingress和一份显式test-service `/self` ingress；
- `skiff.test.ingressUrl`值精确来自动态runner ingress；
- `maxConcurrency: 1`拒绝；
- authored保留config覆盖拒绝；
- ingress缺失或带path时在网络前拒绝；
- 父dispatch frame使用真实ingress origin。

未运行完整workspace gate、stable/live/network/Mongo/OAuth/browser；真实Router/Runtime合流探针由T负责。

## 5. Actual write set

```text
test-runner/src/canonical_package.rs
test-runner/src/lib.rs
test-runner/src/package_test_assembly.rs
test-runner/src/package_test_assembly/http_entry.rs
test-runner/src/package_test_assembly/profile.rs
test-runner/src/runtime_execution.rs
test-runner/src/runtime_execution/tests/orchestration.rs
test-runner/tests/package_service_contract_deployment.rs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-K-test-runner-http-entry-closure.md
  P5-F445H-I7-P8-K-test-runner-http-entry-closure-result.md
```
