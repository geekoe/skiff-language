# P5-F205：旧 Package 根名称遮蔽修复结果

状态：Completed

## 直接父任务

- `P5-F205-legacy-package-root-shadowing.md`

## 首因

根引用收集器只按表达式的字段链头部判断 `root` / `package`，没有保存词法绑定信息。因此：

- 裸局部变量 `package` 被误认为已删除的 Package 根；
- `packageArtifactPut(package)` 中的实参也被误报；
- 局部值的正常成员访问 `package.id` 与旧的未绑定 `package.<module>.<symbol>` 无法区分。

## 修复

`RootRefResolver` 与只读的 `RootRefCollector` 现在使用相同的词法作用域规则：

- 函数参数和模块级 value 声明可遮蔽 `package`；
- `const` 绑定从初始化表达式之后开始生效，并持续到当前块结束；
- `if` / transaction 等嵌套块退出时恢复外层环境；
- `for`、`match` pattern 和 DB lease binding 只在各自 body 内生效；
- 已绑定 `package` 的字段/成员链按普通表达式继续遍历；
- 裸的 `package` 不再作为旧根链处理；
- 未绑定的 `package.<...>` 在任意嵌套表达式中仍明确返回
  `RemovedPackageSyntax`。

类型位置中的旧 `package.<module>.<symbol>` 不属于 value 词法遮蔽，继续 fail closed。

## 验证

- `cargo test -p skiff-compiler-source root_refs --no-fail-fast`
  - 19 passed；
  - 新增裸局部、实参、成员访问、参数遮蔽、嵌套作用域恢复和嵌套调用负例。
- Registry 保留自然源码：
  - `const package = root.model.PackageArtifact { ... }`
  - `packageArtifactPut(package)`
- `npm run test:registry-source`
  - 4 passed。
- 使用 canonical std bootstrap 后执行真实 Registry `package build`：

  ```bash
  cargo run --locked --quiet \
    --manifest-path test-runner/Cargo.toml \
    --bin skiff-package-service-smoke-fixture -- \
    --bootstrap-only \
    --artifact-root <temporary-artifact-root> \
    --environment skiff-f205-registry \
    --platform-source-root /Users/geek/workspace/skiff-p5-f205

  node scripts/skiff.mjs package build \
    /Users/geek/workspace/skiff-packages-phase-05-integration/registry \
    --artifact-root <temporary-artifact-root> \
    --json
  ```

  真实 Registry 自然源码通过 root-reference validator 并成功生成 PackageArtifact、
  ServiceContract 和 ServiceDeployment；Service API 为 20 Available / 0 Package-only。
- 继续执行真实 Registry package tests：

  ```bash
  node scripts/skiff.mjs test \
    /Users/geek/workspace/skiff-packages-phase-05-integration/registry \
    --artifact-root <temporary-artifact-root> \
    --require-tests
  ```

  隔离测试设施在编译 Registry 之前、初始化 MongoDB 后启动 Router 失败：

  ```text
  router exited after start with 1
  error: isolated runtime supervisor exited while initializing MongoDB
  ```

  独立 `package build` 已证明本任务的 validator 阻断关闭；上述错误是测试设施启动阶段的下一项
  独立阻断，尚未进入 Registry fixture。整个验收没有连接或修改共享 stable instance。
- `cargo check --workspace`：通过。
- `git diff --check`：通过。
