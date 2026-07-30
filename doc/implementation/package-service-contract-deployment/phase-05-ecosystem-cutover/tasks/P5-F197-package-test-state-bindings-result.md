# P5-F197：Package Test Database State Bindings 结果

状态：Completed

## 直接父任务

- `P5-F197-package-test-state-bindings.md`

## 交付

- package-test fixture 从 exact `PackageArtifact` 闭包的 typed
  `runtimeRequirements.state` 生成完整 `StateBinding`，不再复制 base deployment 的 ambient
  state，也不按 package 名或约定 key 猜测。
- 每次真实 test run 生成独立 run scope；namespace 由 run scope、typed kind 和非 database
  requirement key 做带版本域分隔的 SHA-256 投影，使用 Mongo-safe `skiff_pt_<40 hex>`。
- 当前一个 activation 只有一个 database capability，因此同一 test run 的多个 database
  requirement key 显式绑定同一个 test-owned database namespace；不同 run 使用不同 namespace。
  非 database state requirement 仍按 exact key 隔离。
- Runtime DB provider 新增 typed `state_namespace` 输入。Mongo database 不再继续按 service id
  隐式选择，而是消费 `ServiceDeployment -> RuntimeAssembly -> StateBinding.namespace`；service id
  仍独立用于 encrypted storage identity。
- Runtime 接受多个 database requirement key 指向同一个 namespace，拒绝一个 activation
  同时指向多个 database namespaces。

## 验证

通过：

```text
cargo test -p skiff-deployment --no-fail-fast
50 passed

cargo test -p skiff-test-runner --test package_service_contract_deployment --no-fail-fast
17 passed; 1 ignored

cargo test -p skiff-runtime-service-db \
  service_db_runtime_uses_typed_state_namespace_as_database_name
1 passed

cargo test -p skiff-runtime-host loader::assembly_admission --no-fail-fast
26 passed

cargo test -p skiff-runtime-package-test --no-fail-fast
5 passed

cargo check --workspace
passed

git diff --check
passed
```

新增 fixture 覆盖 database + queue 多 state、typed kind、同 scope 可重现及不同 run namespace
隔离。deployment 原有 projection matrix 覆盖缺失、多余和错误 kind，仍全部通过。

真实 `http-session,track` 使用临时 artifact root 和隔离 instance 验证：

- std seed、http-session publish/build、track build 通过；
- wrong exact http-session ref 负例按预期 fail closed；
- 原 `missing state binding ...` 已消失；
- http-session 继续进入 Runtime 后，在后续既有 whole-assembly Link 阶段被拒绝：
  `AssemblyActivationRejected ... rejected activation during link`。Runtime 日志确认 stage 为
  `Link`，不是本任务修改的 state projection 或 DB context `Admit` 阶段。该后继 blocker 交由
  package revalidation 链继续处理。

没有访问 stable instance，没有使用 ambient/stable database，也没有 push。
