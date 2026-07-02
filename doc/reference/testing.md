# Skiff Testing Reference

本文负责测试源码、测试发现、runner 模式、package 测试和 production artifact 边界。本文不负责
具体 CLI flag、测试进程编排、live secret 管理或 runner 的实现细节。

## 1. Testing Surface

Skiff 只保留一种测试用例语义：`test` block。unit、integration 和 live smoke 不是不同
语法，而是 runner 模式和 effect policy 的差异。

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
- 它不改变 runner mode、network permission、config 注入或 live key policy。
- 它只接受 literal bool，不接受表达式、config 或运行时条件。

## 3. Friend Test Files

同目录 test-only 文件可以成为某个生产文件的 friend 文件。friend 是 test-runner 建立的
白盒测试作用域：它允许 test-only 文件访问对应生产文件中未进入 public API 的 top-level
declarations。

匹配规则：

- `foo.test.skiff` 是 `foo.skiff` 的 friend。
- `foo.*.test.skiff` 也是 `foo.skiff` 的 friend。
- friend 关系只由同目录文件名前缀决定，目录名没有测试级别语义。
- friend 权限只覆盖对应生产文件，不因 import、目录相近或 package-internal `root.*`
  规则扩展到其他生产模块。
- 无法在同目录匹配唯一生产文件的 test-only 文件是普通测试文件，只能访问 production
  public API 和外部 dependency 的 public API。

文件名中的 flavor，例如 `live`、`fuzz`、`bench`，只是组织约定，不改变测试语义。

## 4. Test Discovery

测试必须显式启动。runner 输入可以是普通 source file、test-only source file 或目录。

普通 source file 输入：

- 运行同目录 matching friend test files。
- `defaultRun false` 的 friend 文件不会被默认运行。

test-only source file 输入：

- 只运行该文件中的测试。
- 若它是 friend 文件，测试编译建立 friend 作用域。
- 显式指定文件时不受 `defaultRun false` 跳过。

目录输入：

- 递归发现 `*.test.skiff`。
- 跳过 `defaultRun false` 文件。
- 跳过 generated / dependency 目录，例如 `target`、`node_modules` 和 dot directory。

## 5. Runner Modes

测试级别由 runner mode 和 effect policy 决定，不由语法、目录名或文件名决定。

VM / unit mode：

- 使用 compiler test VM，不启动 router / runtime 进程。
- 不访问真实网络或外部服务。
- 外部 effect 必须由 test double 替换；缺失 double 必须失败。
- 每个 test case 使用独立 VM / double registry。

Runtime / integration mode：

- 使用真实 Skiff runtime 语义执行测试。
- runner 负责构造临时 service activation / request frame。
- package 测试需要 runtime 时，runner 自动生成临时 test service / activation。
- config 由 runner 注入 resolved config；package 不读取 ambient environment。

Live smoke：

- 是 runtime / integration mode 的显式、昂贵用法。
- 应使用 `defaultRun false` 并通过文件路径运行。
- 没有 live key 时应 skip，而不是失败。
- 只验证真实外部服务的少量关键路径，不替代 unit / integration 覆盖。

## 6. Package Tests

package 测试归 package 所有，不需要通过 sample service 承载。

规则：

- package test helper 不进入 package production assembly。
- package integration test 可以是普通 test-only source file。
- 需要 runtime 的 package 测试由 runner 构造临时 service / activation。
- friend test 是白盒测试；非 friend test-only 文件按黑盒边界检查，只能访问 production
  public API。
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

- 改生产文件，运行对应 source file 输入。
- 改 test-only 文件，运行该 test-only 文件。
- 改 package public API、manifest 或 shared helper，运行受影响 package 的测试。
- 改 runtime effect、config、HTTP 编码、router activation，运行相关 integration 测试。
- live smoke 只在用户显式要求、nightly 或 release 验证流程中运行。

Runner flag 只控制执行宿主和 effect policy，不改变测试源码语义，也不把 `defaultRun false`
文件加入目录默认发现。

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
