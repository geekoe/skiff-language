# P5-F08：Host Test-Runtime Seam Repair

## 输入、owner与限制

- 依赖：D09完成；F04 implementation checkpoint `fed409ba374a85891dc9834179a7cc8bee4ae258` / tree
  `b65ef0b67e13dea4a690671b7a5b4b7bc8b3efd0`已合入integration，root已完成唯一shared lock checkpoint。
- 使用独立worktree/branch，只提交一个clean commit，不merge/push。PASS只解锁R08，不给F04/R02 verdict。
- production owner限于`runtime/host/src/artifact_cache/mod.rs`与`runtime/host/src/host/{control_plane,mod,
  package_test_entry,register_mapper,request_entry,route_registry,router_session,runtime_host}.rs`及直接tests；
  `package_test_entry.rs`应删除，`route_registry`只删失去调用者的package-test revision helper。
- 不改Router/compiler/std/deployment/test-runner/runtime-package-test/driver、assembly admission/provisioning lifecycle、
  request/transport schema、typed WS、manifest或root lock；不加compat alias、host-local assembly推导、synthetic
  service/context、第二cache/admission owner。

## 完成态

1. Host不再import或消费`skiff_runtime_package_test`、`PackageTestBuildSelection`、
   `PackageTestDispatchSelection`、`LoadedPackageTestRuntimeProgram`或template cache；删除legacy
   `package-test.start`接收、排队、cache、local config与synthetic execution。
2. `runtime.capabilities`与legacy service registration不再声明`packageTestDispatch: true`；request cancellation只处理
   normal request supervisor，不保留pending package-test executor。
3. activation reply使用`RuntimeToRouter`方向的`encode_assembly_activation_frame`；activation command使用
   `RouterToRuntime`方向的`decode_assembly_activation_frame`。不复制codec或修改wire。
4. canonical assembly activation、active route与production Host ingress保持既有owner，F08不实现F03C的startup、
   lifecycle、drain或WS语义。

## 验证

```bash
cargo check --locked -p skiff-runtime-host
cargo test --locked -p skiff-runtime-host runtime_capabilities_registers_without_loaded_services
cargo test --locked -p skiff-runtime-host --test active_runtime_assembly
cargo test --locked -p skiff-runtime-transport assembly_activation_frame
cargo test --locked -p skiff-runtime-package-test --test package_artifact
git diff --check
```

每个filter必须非零。反向搜索`skiff_runtime_package_test|PackageTest(Build|Dispatch)Selection|
LoadedPackageTestRuntimeProgram|PackageTestRuntimeTemplateCache`在`runtime/host/src`为零，且Host不再出现
`package_test_dispatch: true`或`"package-test.start"`。回报25-error disposition、capability/codec矩阵、exact
source/commit/tree、single commit/clean/lock与extra-review。
