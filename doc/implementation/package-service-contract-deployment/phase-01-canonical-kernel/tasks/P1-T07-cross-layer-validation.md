# P1-T07：跨层 Artifact Reference Validation

## 目标

让compiler写出的ServiceUnit/PackageUnit pointer与当前serviceAssembly内容身份在runtime/router load path
严格验证，并把跨语言重复hash实现收敛到Rust canonical owner或CLI。

## 依赖与 worktree

- 依赖P1-T02、P1-T02A、P1-T03、P1-T05、P1-T06。
- 从包含全部前置提交的phase checkpoint建task worktree。
- 建议branch：`codex/package-service-p1-t07-cross-layer-validation`。

## 架构决定

1. compiler写出的pointer就是唯一合法wire。ServiceUnit pointer必须同时包含
   `schemaVersion`/`unitIdentity`/`unitHash`/`unitPath`；PackageUnit pointer必须同时包含
   `schemaVersion`/`packageId`/`version`/`buildIdentity`/`abiIdentity`/`unitHash`/`unitPath`。
   runtime、router和scripts不得把字段降成optional，也不得接受`artifactPath`、`path`或path-only旧形状。
2. Service assembly identity与ServiceUnit/PackageUnit内容校验归`artifact-identity`。compiler emission和Rust
   runtime直接调用crate；TypeScript只在load/reload时调用CLI，不实现stable stringify、preimage或hash。
3. 扩展已有`runtime-program-build-id`CLI事务：一次调用验证service assembly、ServiceUnit pointer和全部
   PackageUnit pointer/内容闭包，并返回typed validation result与dynamic build id。不要新增一次独立CLI
   往返；调用频率保持每次artifact load/reload至多启动一个CLI进程，不进入request dispatch hot path。
   同一批候选可来自不同artifact root，因此每个候选携带自己的root；CLI返回load path可直接采用的已验证
   结果，TypeScript不得在校验后重新读取同一内容制造TOCTOU窗口。
4. ServiceUnit继续使用canonical service-unit projection/hash。PackageUnit的`unitHash`是完整unit JSON的
   canonical content hash，不能误用build identity projection；同一次校验还必须验证typed schema、坐标、
   publication/local ABI/build identity及pointer一致性。assembly identity也不能与dynamic build id混用。
5. artifact-relative path在任何`join`/read前由共享Rust owner验证。空路径、绝对路径、`.`、`..`、反斜杠
   和非canonical坐标路径全部fail-closed；即使宿主平台把反斜杠视为普通字符也必须拒绝。
6. load/reload的任一CLI、identity、pointer、path或闭包校验失败都使本次候选整体失败；reload保留旧的
   active snapshot，不能部分接受新service或package unit。

## 完成态

1. ServiceUnit pointer完整保留并校验`unitIdentity`/`unitHash`；PackageUnit pointer不仅透传`unitHash`，还
   对加载内容recompute/validate schema、coordinates、publication/build/local ABI identity和完整JSON hash。
2. 当前serviceAssembly canonical content projection/hash只有`artifact-identity`一个Rust owner；compiler
   emission与runtime host直接调用crate，router在artifact reload/load时通过上述单次identity CLI事务调用。
3. Router删除`serviceAssemblyHashInput`/stableStringify身份算法；CLI输出typed result和稳定error，调用只
   发生在artifact load/reload，不进入request hot path。
4. scripts/dev sync保留完整pointer并调用同一CLI事务做内容验证，不复制preimage；sync产物缺字段或引用
   内容不一致时fail-closed。
5. artifact-relative path必须在读取前fail-closed；Rust和TS至少共享同一cross-system fixture覆盖绝对路径、
   空/`.`/`..`、反斜杠、identity stem与owner coordinate mismatch。本阶段不建立共同PublicationId领域类型。
6. single-source checker扫描compiler/runtime/router/scripts中的production hash/preimage定义，允许canonical
   owner和CLI adapter，拒绝第二实现。
7. T03新semantic metadata wire shape在router/runtime loader与runtime package-test消费者同步切换；compiler
   PackageUnit/package-test producer已由T03完成，T07不得再次定义或迁移该shape。不保留旧字段fallback。
8. runtime/package-test删除本地`package_implementation_links_identity`与prefix，改调T06的canonical
   owner；checker删除“runtime留给T07”的临时放行，并把该identity纳入全局exclusive owner检查。
9. compiler driver/test-runner的真实package-test调用传入生产与依赖package的resource blobs，删除
   `Vec::new()`占位；非空resource refs端到端可运行，missing/extra/hash/path不一致仍fail-closed。

## 实现顺序与边界

1. 先在`artifact-identity`建立service assembly projection/identity、完整pointer/closure validation与安全
   artifact path API，并用Rust测试和cross-system fixture锁定wire。
2. 再迁移compiler emission与runtime loader/package-test；此时删除Rust侧重复算法和optional pointer字段。
3. 最后迁移router、scripts与fixtures，通过现有CLI adapter完成同一次load/reload校验，再收紧checker。

不要让router/scripts shell出CLI后又在TypeScript复算结果；不要为了减少fixture修改而保留字段alias或
optional fallback。`runtime-program-build-id`只验证已选择的artifact候选，不负责dispatch或active snapshot
切换。

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
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/skiff-dev-sync.mjs --check-sync --root compiler/tests/fixtures/router-websocket-fixture
git diff --check
```

允许开发中用单crate或单test selector；提交前至少完成上述直接受影响gate。全量`cargo fmt`若只命中仓库已知
baseline，必须列出文件并对本任务文件做targeted rustfmt。测试必须覆盖content tamper、pointer字段缺失/篡改、
unit hash mismatch、path traversal/backslash、CLI unavailable/failure、reload保留旧snapshot、非空package
resource，以及compiler/runtime/router parity。

## 自验收与回报

反向搜索TS/Rust service assembly preimage、runtime implementation-links helper/prefix、optional pointer字段、
未验证unitHash、raw artifact path join、package-test`Vec::new()` resource placeholder和旧semantic metadata
field；说明每个剩余命中。提交自验收矩阵、CLI调用频率证据、reload原子性证据与commit。

## 停止条件

- 现有CLI调用点无法在不增加request-path调用的前提下获得完整assembly/unit closure。
- PackageUnit `unitHash`现有wire与“完整canonical JSON hash”事实不一致。
- compiler写出的pointer在合法production路径缺少本任务要求字段。
- resource blob只能通过引入第二套PackageUnit builder才能传给test-runner。

出现停止条件先回报并升级前置任务；不得用optional字段、旧alias或本地hash临时绕过。
