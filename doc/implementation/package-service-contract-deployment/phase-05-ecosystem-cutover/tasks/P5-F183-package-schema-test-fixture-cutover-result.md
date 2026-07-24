# P5-F183：Package Schema 测试夹具硬切结果

状态：Completed

## 结果

- compiler integration 的共享 Package graph 夹具不再只传 artifact coordinate：
  - 普通依赖按 manifest 的有效 alias，从真实已编译 `PublishedPackageArtifact` 的
    schema index、own records、build 与 ABI 构造 `ResolvedPackageSchema`；
  - compiler-owned std 使用同一条真实 Package schema 链和固定 `std` alias；
  - ServiceContract 自引用 Package 类型的夹具先从真实 `api.yml` 与类型声明编译
    schema-only seed，再显式注入最终 compile，不再手写 descriptor 或 record。
- test-runner 的 Package project 与 test overlay 把调用者已打开的
  `CanonicalArtifactStore` 传给 compiler；无依赖的直接 compile helper 仍不安装
  resolver，缺 resolver 的负例继续失败关闭。
- service authoring 在交给 deployment 前，按生成 ServiceContract 的
  `packageTypeRequirements` 选择精确 schema record closure；不会再把未被合同引用的
  依赖类型记录作为多余输入传入，也没有放宽 deployment 的 exact-closure 校验。
- runtime/package-test 的篡改负例更新为检查精确的 Package ref/content mismatch 及
  篡改版本证据，没有弱化错误类别。
- PackageArtifact JSON 字段、移除的 `native type` 语法、Actor 公开符号数量以及 std
  Package build identity 等旧断言/golden 已按当前输入语义更新。
- 保留了“无关 Package 类型不改变 service protocol identity”的现有测试；Package
  build identity 仍随 Package 输入变化。

## 失败关闭覆盖

- 缺 schema bundle/store resolver 继续拒绝；
- schema index/record owner、stable key、type identity、Package build 与 ABI 的既有
  篡改负例继续通过；
- contract dependency 缺 record、多余 record、未使用 record 和 closure 不精确继续拒绝；
- test overlay 不会从 source/manifest 重建或猜测 schema；
- 无 schema requirement 的 runtime/package-test 夹具没有制造虚假 schema。

## 验证

- compiler integration 中原有
  `no resolved schema or canonical store resolver` 失败：0
- `cargo test -p skiff-compiler --test file_ir_execution_type_representation`
  - 2 passed
- `cargo test -p skiff-compiler --test service_conformance`
  - 11 passed
- `cargo test -p skiff-compiler --test package_imports`
  - 7 passed
- `cargo test -p skiff-compiler --test package_std_schema`
  - 7 passed
- `cargo test -p skiff-runtime-package-test --no-fail-fast`
  - 5 passed
- `cargo test -p skiff-runtime-loader --lib --no-fail-fast`
  - 10 passed
- `cargo test -p skiff-test-runner --test package_service_contract_deployment --no-fail-fast`
  - 15 passed，1 ignored
  - 另 1 项被独立的 std receiver 静态类型回归阻断；不再存在 schema resolver、
    extra-record closure 或 build golden 失败
- `cargo check --workspace`
  - passed
- `git diff --check`
  - passed

完整 compiler 聚焦组合不再出现缺 schema resolver；剩余失败属于并行的 F184
actor/普通 impl 字段解析、HTTP boundary 与 interface fixture 回归，不在本任务中通过
恢复 fallback 或修改期望掩盖。
