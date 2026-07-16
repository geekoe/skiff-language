# P1-T07：跨层 Artifact Reference Validation

## 目标

让compiler写出的ServiceUnit/PackageUnit pointer与当前serviceAssembly内容身份在runtime/router load path
严格验证，并把跨语言重复hash实现收敛到Rust canonical owner或CLI。

## 依赖与 worktree

- 依赖P1-T02、P1-T03、P1-T05、P1-T06。
- 从包含全部前置提交的phase checkpoint建task worktree。
- 建议branch：`codex/package-service-p1-t07-cross-layer-validation`。

## 完成态

1. ServiceUnit pointer完整保留并校验`unitIdentity`/`unitHash`；PackageUnit pointer不仅透传`unitHash`，还
   对加载内容recompute/validate schema、coordinates、build/local ABI identity和hash。
2. 当前serviceAssembly canonical content projection/hash只有`artifact-identity`一个Rust owner；compiler
   emission与runtime host直接调用crate，router在artifact reload/load时调用identity CLI。
3. Router删除`serviceAssemblyHashInput`/stableStringify身份算法；CLI输出typed result和稳定error，调用只
   发生在artifact load/reload，不进入request hot path。
4. scripts/dev sync对identity只解析canonical prefix/path并调用同一CLI进行内容验证，不复制preimage。
5. artifact-relative path必须在读取前fail-closed；Rust和TS至少共享同一cross-system fixture覆盖绝对路径、
   `..`、分隔符、identity stem与owner coordinate mismatch。本阶段不建立共同PublicationId领域类型。
6. single-source checker扫描compiler/runtime/router/scripts中的production hash/preimage定义，允许canonical
   owner和CLI adapter，拒绝第二实现。
7. T03新semantic metadata wire shape在router/runtime loader与runtime package-test消费者同步切换；compiler
   PackageUnit/package-test producer已由T03完成，T07不得再次定义或迁移该shape。不保留旧字段fallback。

## 写入范围

- artifact-identity CLI与service assembly/pointer validation modules。
- compiler emission/index pointer写出。
- runtime loader/host、runtime/package-test与test-runner直接consumer/fixtures。
- router artifacts/identity CLI adapter、scripts dev-sync及直接tests/fixtures。
- single-source checker扩展。

不要改变router dispatch、service runtime执行、ServiceUnit code ownership、assembly schema业务字段或remote
fallback。

## 验证

```bash
cargo fmt --all -- --check
cargo test -p skiff-artifact-identity -p skiff-compiler-emission
cargo test -p skiff-runtime-loader -p skiff-runtime-host
cargo test -p skiff-runtime-package-test -p skiff-test-runner
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- dynamic-build-id-parity artifacts artifact-reload
node scripts/check-artifact-identity-single-source.mjs
node scripts/skiff-dev-sync.test.mjs
git diff --check
```

若scripts测试入口名称不同，使用`rg`确认权威命令并记录。测试必须覆盖content tamper、pointer tamper、
unit hash mismatch、path traversal、CLI unavailable/failure fail-closed和compiler/runtime/router parity。

## 自验收与回报

反向搜索TS/Rust service assembly preimage、未验证unitHash、raw artifact path join和旧semantic metadata field；
说明每个剩余命中。提交自验收矩阵、CLI调用频率证据与commit。
