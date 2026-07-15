# Skiff Testing Reference

本文负责测试源码、测试发现、runtime 执行语义、package 测试和 production artifact 边界。本文
不负责具体 CLI flag、测试进程编排、live secret 管理或 runner 的实现细节。

## 1. Testing Surface

Skiff 只保留一种测试用例语义：`test` block。unit、integration 和 live smoke 不是不同
语法或执行宿主，只是测试范围、effect policy 和 runtime target ownership 的差异。

规则：

- `test` block 只允许出现在 `*.test.skiff` 文件中。
- `assert` 只允许出现在 `test` block 中。
- 生产文件是所有不以 `.test.skiff` 结尾的 `.skiff` 文件。
- 生产文件中的普通 declaration 都进入生产编译产物，即使它只被测试使用。
- test-only declaration 只参与测试编译，不进入 production artifact、package assembly、
  service assembly、public API surface 或 config metadata。

## 2. Test-Only Source

`*.test.skiff` 是测试专用 source file。它可以包含测试用例、helper、fixture type、测试
专用 import，以及可选的 `test defaultRun false` directive。

`test defaultRun false` 是文件级测试发现 directive：

- 默认值是 `true`。
- 只影响目录输入的默认发现。
- 显式指定该 test-only 文件时，runner 必须运行它。
- 它不改变 runtime target ownership、network permission、config 注入或 live key policy。
- 它只接受 literal bool，不接受表达式、config 或运行时条件。

## 3. Package-Local Visibility

source file 不是当前 package / service 内的可见性边界。runner 构造测试编译时，选中的
test-only source 与当前 publication 的全部 production sources 共享同一个 all-symbol
`root.*` index；测试可以访问任意 production source file 的顶级 function、type、alias、
interface、const 和 db object，无论它是否进入 public API。

这不是 test-runner 额外授予的白名单，也不依赖测试文件名：

- `foo.test.skiff`、`foo.live.test.skiff` 和其他 `*.test.skiff` 没有对应 production file。
- 文件名中的 `live`、`fuzz`、`bench` 只是组织约定，不改变可见性。
- production 编译使用同样的当前 publication all-symbol 规则，但它的 source set 不包含
  test-only files，因此 production code 不能引用 test-only helper。
- 外部 package 仍只通过 published public API / exports 可见；测试编译不会把 dependency
  private symbol 加入当前 publication 的 `root.*` index。

测试选择和符号可见性是两件独立的事。选择单个测试文件只决定运行哪些 test cases，不会
缩小或扩大当前 publication 的 production symbol set。

## 4. Test Discovery

测试必须显式启动。runner 输入可以是普通 source file、test-only source file 或目录。

普通 source file 输入：

- 运行该 source file 所属 service / package 中默认发现的测试。
- 与目录输入一样跳过 `defaultRun false` 文件；不按 source file 名称匹配 test-only 文件。

test-only source file 输入：

- 只运行该文件中的测试。
- 显式指定文件时不受 `defaultRun false` 跳过。
- 测试编译仍包含所属 service / package 的全部 production 顶级符号。

目录输入：

- 递归发现 `*.test.skiff`。
- 跳过 `defaultRun false` 文件。
- 跳过 generated / dependency 目录，例如 `target`、`node_modules` 和 dot directory。

## 5. Runtime Execution And Effect Policy

所有 Skiff 测试源码都由 `skiff-test-runner` 编译，并在真实 Skiff runtime 进程中执行。Skiff
不提供 compiler VM / unit 执行模式；unit、integration 和 live smoke 只描述测试范围，不改变
执行语义。测试级别由 effect policy 和 runtime target ownership 决定，不由语法、目录名或
文件名决定。

非 live 测试：

- 普通 `skiff test <path>` 为整个命令创建一套隔离 router / runtime，并在其中运行全部 case。
- 仓库 canonical Skiff 源码套件为整个 registry plan 创建一套隔离 router / runtime，并在所有
  registry entry 之间复用该进程。
- 不访问真实网络或外部服务；外部 effect 必须由 test double 替换，缺失 double 必须失败。
- runner 负责构造临时 service activation / request frame；package 测试由 runner 自动生成
  临时 test service / activation。
- config 由 runner 注入 resolved config；package 不读取 ambient environment。
- runtime 进程复用不扩大可变状态生命周期。每个 case 的 double registry、临时 artifact、
  service activation 和测试状态仍按 runner isolation contract 清理。

Live smoke：

- 同样在真实 runtime 进程中执行，但 target 由调用者显式提供和拥有，并允许显式授权的外部
  effect。
- 应使用 `defaultRun false` 并通过文件路径运行。
- 没有 live key 时应 skip，而不是失败。
- 只验证真实外部服务的少量关键路径，不替代 unit / integration 覆盖。

## 6. Package Tests

package 测试归 package 所有，不需要通过 sample service 承载。

规则：

- package test helper 不进入 package production assembly。
- package integration test 可以是普通 test-only source file。
- 需要 runtime 的 package 测试由 runner 构造临时 service / activation。
- 所有 package test-only 文件都可访问当前 package 的 production 顶级符号；跨 package
  访问仍受 dependency public API / exports 限制。
- package public API、`package.yml` 或 shared helper 变化，应运行受影响 package 的相关
  test-only files 或目录。

Package 不是远程 service。package 测试应验证本机 ABI、source wrapper、effect metadata 和
runtime host boundary，不应伪装成 service-to-service 测试。

## 7. Test Doubles

测试替身按 stable target id 匹配。double 可以匹配 `std.*` host-backed API、普通 package
wrapper 或 service operation target。

规则：

- double 可以声明 expected request subset。
- double 必须返回 schema-closed payload，或抛标准 `ErrorPayload` leaf。
- double 执行仍参与 effect summary。
- mock 不能绕过 `concurrent` effect conflict 检查。
- double registry 在每个 test case 结束后清理。

## 8. AI / CI Selection

AI 和 CI 不需要测试配置文件来决定默认测试。它们按改动范围显式选择文件或目录。

原则：

- 改生产文件，运行所属 service / package 目录，或显式选择受影响的 test-only 文件。
- 改 test-only 文件，运行该 test-only 文件。
- 改 package public API、manifest 或 shared helper，运行受影响 package 的测试。
- 改 runtime effect、config、HTTP 编码、router activation，运行相关 integration 测试。
- live smoke 只在用户显式要求、nightly 或 release 验证流程中运行。

Runner flag 只控制 runtime target ownership 和 effect policy，不改变测试源码语义，也不把
`defaultRun false` 文件加入目录默认发现。非 live 与 live 都不切换到 compiler VM。

## 9. Production Artifact Boundary

production build 必须满足：

- 生产文件中出现 `test` block 是编译错误。
- `*.test.skiff` 不进入 production source set。
- test-only code 不进入 file artifact bytecode、service assembly 或 package assembly。
- test-only config reads 不进入 production config use metadata。
- test-only declarations 不进入 production package API 或 service protocol surface。
- test-only helper 不影响 package / service identity。
- test-only `root.*` reference 不参与 production root reference validation。
- `test defaultRun` directive 不进入 production artifact。

Test assembly 不是 deployable assembly。
