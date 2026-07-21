# P5-F11：Std Exact Callable Effects Repair

## 输入、owner与限制

- 输入：D13完成；exact code integration `c06e1152463c7206a1a19c055294c368e3ae5fac` / tree
  `7a02d8c90c89e1f91c2f50d91fd26791291e051b`，已包含F04B runner disambiguation。
- 独立worktree/branch，一个clean commit，不merge/push。R12 PASS只恢复F04真实Host probe。
- 单一owner覆盖artifact-model exact semantics、compiler/source resolved target facts、lowering facts消费、runtime native/
  receiver registry parity及直接tests；test-runner只可补完整std projection/assembly断言。
- 不改boundary eligibility、std source/registry、runner policy、artifact schema/identity、deployment、Router/Host fixture、
  manifest/Cargo.lock、F05或stable。

## 完成态

在D13列出的10个exact native keys与3个receiver identities上建立production descriptor。既有四个string descriptor
原样保留。source resolved target facts成为唯一target identity owner；lowering消费并核对facts，不再独立猜
Date/Duration receiver。artifact-model仍是稀疏callable-semantics数据owner，runtime按exact key验证signature、required
context与真实handler/route；compiler/deployment/runner保持fail-closed consumers。

- `core.date.now`：Time context，`NativeRegistry/date_now`；
- duration constructors、number与5个crypto：无context，对应exact NativeRegistry handler；
- `std.time.sleep`：Time context，`TimeNativeDispatch/sleep_for_millis`，只标`may_suspend`；
- 三个receiver：canonical `receiver:*@1` target，runtime/eval `ReceiverMethodDispatch`，pure scalar result。

unknown descriptor、dynamic/first-class target、mutable/unknown receiver及file/http/websocket capability native仍产生原
fail-closed effects。helper package的direct mutation仍因`WritesCallerReachable`不可作为detached boundary；consumer/test
projection仍Available，最终Host业务值不得变化。

## 验证

```bash
cargo test --locked -p skiff-artifact-model native_signature::tests::
cargo test --locked -p skiff-artifact-model builtin_receiver_ops::tests::
cargo test --locked -p skiff-compiler-source callable_effects::tests::
cargo test --locked -p skiff-compiler-source resolved_call_targets::tests::
cargo test --locked -p skiff-compiler-lowering source_file_lowering::tests::
cargo test --locked -p skiff-runtime-native native_callable_semantics_registry
cargo test --locked -p skiff-runtime-native dispatch::tests::native_signature_registry_shared_targets_are_runtime_reachable -- --exact
cargo test --locked -p skiff-runtime-native registry::tests::std_crypto_native_targets_dispatch -- --exact
cargo test --locked -p skiff-runtime-native registry::tests::date_native_targets_dispatch -- --exact
cargo test --locked -p skiff-runtime-native registry::tests::duration_native_targets_dispatch_erased_milliseconds -- --exact
cargo test --locked -p skiff-runtime-native registry::tests::std_number_safe_integer_natives_dispatch -- --exact
cargo test --locked -p skiff-runtime-native dispatch::tests::std_time_sleep
cargo test --locked -p skiff-runtime-eval receiver_methods::tests::date_receiver_methods_dispatch -- --exact
cargo test --locked -p skiff-runtime-eval receiver_methods::tests::duration_receiver_methods_dispatch_erased_milliseconds -- --exact
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment official_platform_package_is_compiled_as_the_selected_source_root -- --exact
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment fresh_helper_mutation_then_detached_service_call_projects_and_assembles -- --exact
node scripts/run-skiff-tests.mjs
git diff --check
```

每个exact filter必须非零。完整std探针必须恰好发现11 cases全部Available并成功assembly；负例保持forged Available、
unknown/dynamic/mutable/file/http/websocket fail closed。最终Node命令必须真实进入Host fixture，若暴露下一blocker只报告
首个证据、不越界。回报exact key→facts→lowering→runtime矩阵、commit/tree/lock、single clean、reverse与extra-review。

## R12 acceptance record

F11 candidate `a9ef444d258497224f59633e29759ee185031ee7` / tree
`5a497262f1a2cf29f81b4e12a958edc2a729cbe1`由独立R12判定PASS并合流为`2d74b2c`。13项exact矩阵、source
facts唯一owner/lowering parity、runtime route/context、完整std 11 Available/assembly与全部负例通过；lock不变。
Node已越过effects blocker，但Runtime在native前拒绝Router flat request header，转交D14，不归因F11。
