# P5-F268 Public repository test service migration result

## 结果

完成。

Skiff 仓库的规范源码测试与 `skiff-packages` 的官方 Package 测试已迁移为显式
`kind: test` 服务。迁移后的测试服务通过普通 Package/Service 编译、artifact、链接和
runtime 路径运行，不再依赖被测 Package 内的测试源码叠加。

本任务分别提交到两个仓库：

- Skiff：`2350f1c`（`fix(test): close public test service migration`）；
- skiff-packages：`b8c6a41a`（`test: migrate packages to test services`）。

未 push，未操作共享 stable instance。

## 迁移结果

`skiff-packages/tests/` 下现有六个普通测试服务：

- `aliyunoss`
- `http-session`
- `openai`
- `openai-live`
- `registry`
- `track`

每个测试服务都有自己的 `package.yml`、`service.yml`、
`config.skiff-test.yml` 和普通 artifact 输入；被测 Package 通过精确依赖引入，并仅在
`kind: test` 服务中使用 `topLevel` 权限。测试源码使用
`alias/source.module.name` 访问被测顶层符号，未保留 `root.*` 私有访问。

共享配置已经移入普通 profile 配置。五个 HTTP 替身都内联在所属测试块中，并使用精确
`std/http.request` effect 目标。仓库中的 `skiff.test-doubles.json` 已全部删除。

38 个原测试用例均已保留：

- 36 个不依赖真实外部凭据的用例由默认测试命令执行；
- `openai-live` 的 2 个真实外部调用用例归入独立测试服务，默认完成完整编译，但不执行
  网络请求。

## 为真实测试服务补齐的编译与运行路径

迁移暴露了三处此前被旧叠加模型掩盖的缺口，本任务一并关闭：

- 编译器把规范 std artifact 注入依赖分析，使 `std/http.request` effect 与生产代码中的
  `std.http.request` 解析为同一个精确 `PackageCallable` 身份；已有 prelude 原生函数
  仍按原路径处理。
- 测试服务调用 Package 顶层函数时，参数按被调用 Package 的精确公开类型做上下文类型
  检查，不会把结构相同但 Package 身份不同的名义类型误认为同一类型。
- runtime loader 会递归装载 Package schema 的跨 Package 精确闭包，并对缺失记录、身份或
  内容不一致以及跨 Package 环做失败关闭。

这些路径没有引入 std 字符串特判或 runtime 原生 HTTP 旁路。

## 验证

Skiff 任务相关门禁全部通过：

- `cargo check --workspace --all-targets`
- `cargo test -p skiff-compiler-source --lib --quiet`：290 passed
- `cargo test -p skiff-compiler-lowering --lib --quiet`：43 passed
- `cargo test -p skiff-runtime-loader --quiet`：13 unit + 2 integration passed
- `cargo test -p skiff-runtime-eval --lib --quiet`：149 passed
- `cargo test -p skiff-syntax --quiet`：116 passed
- `cargo test -p skiff-compiler --test std_package_imports --quiet`：7 passed
- `cargo test -p skiff-test-runner --test package_service_contract_deployment --quiet`：
  20 passed，1 ignored
- `node scripts/run-skiff-tests.mjs`：std 11、alias 6、Package/Service Host 4，全部通过
- `cargo fmt --all -- --check`
- `git diff --check`

`skiff-packages` 全量官方测试服务通过：

- aliyunoss：6 passed
- http-session：19 passed
- openai：2 passed
- registry：5 passed
- track：4 passed
- openai-live：2 个用例编译通过，按设计不执行真实外部请求
- `node --test scripts/registry-service-source.test.mjs`：5 passed
- `node --check scripts/test-packages.mjs`
- `git diff --check`

## 已知的后续边界

仓库级 `cargo test -p skiff-compiler --no-fail-fast` 仍能发现 9 个不属于 F268 的旧测试目标
失败，集中在旧 std build id、缺少数据库状态的历史 fixture、旧原生 stream/HTTP 预期、
缺少 Package owner 的名义类型 fixture 和未初始化 platform source。F268 使用的编译器、
loader、runtime 与规范测试门禁均已单独通过；这些历史基线没有通过放宽类型或运行规则来
绕过。

F270 继续拥有旧测试模型的最终删除。其封闭清单当前保留 5 个旧 fixture，其中 3 个仍含
`root.*`；它们不在 F268 已迁移的规范/官方测试集合内。整个 Skiff 与 skiff-packages
工作树中已经没有 `skiff.test-doubles.json`。
