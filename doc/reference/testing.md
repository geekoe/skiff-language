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

## 3. Test Service And Visibility

测试源码属于独立 test service，不再作为被测 package 的 source overlay 编译。test service
使用普通 PackageArtifact、ServiceContract、Deployment 和 RuntimeAssembly 格式；不定义
TestServiceArtifact。

`service.yml` 必须声明：

```yaml
id: example.com/widget-tests
kind: test
```

`kind: test` 是 authoring/workflow 约束：

- 只允许 `skiff test` 构建和运行；
- 普通 package publish、service publish/deploy 和 production watch 拒绝 test service；
- artifact、linker、loader 和 Runtime 使用普通格式与执行路径；
- test service 的 config 来自普通 `config.<profile>.yml`；
- 同一个 test service 的 cases 共享配置和 dependency graph，但每个 case 仍有独立 state
  namespace、heap、effect registry 和 execution nonce；
- 需要不同配置时使用另一个 test service 或显式 profile，不提供 per-case config override。

test service dependency 可以声明：

```yaml
packages:
  - id: example.com/widget
    version: 1.0.0
    alias: widget
    access: topLevel
```

`access` 是互斥解析模式：

- 缺省 `public`：只按 dependency `api.yml` public paths 解析；
- `topLevel`：只按精确 implementation artifact 的 source module/top-level symbol index 解析，
  完全忽略该 dependency 的 `api.yml`；
- 不允许 public-first、topLevel-fallback 或两套路径合并；
- `topLevel` 仅允许出现在 `kind: test` service，且不传递到 dependency 的 dependencies。

topLevel 语法为：

```text
<dependency-alias>/<source-module-path>.<top-level-name>
```

`root.*` 始终表示 test service 自己。测试访问被测 package 必须写成例如
`widget/internal.codec.decodeForTest(...)`，避免与本 service 或其他 dependency 冲突。

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

package 测试由归属该 package 仓库的 test service 承载。test service 把被测 package 声明为
精确 dependency；需要内部顶层访问时使用 `access: topLevel`。

规则：

- test helper 只进入 test service artifact，不进入被测 package production artifact；
- 测试通过普通 config、dependency、contract、deployment 和 assembly 机制运行；
- package 内部测试使用 topLevel dependency call，不使用 overlay `root.*`；
- Package 仍不是远程 service；本机 Package call 不得伪装成 service-to-service RPC；
- public API、implementation top-level、manifest 或 shared helper 变化时运行对应 test
  services。

## 7. Inline Test Effects

effect doubles 写在所属 test block 中，不使用外部 `skiff.test-doubles.json`。

规范形态：

```skiff
test "request succeeds" effects {
  std.http.request {
    expect: {
      method: "POST",
      url: "https://example.test",
    },
    respond: {
      status: 200,
      headers: Array.empty<std.http.HttpHeader>(),
      body: bytes.fromUtf8("ok"),
    },
  }
} {
  // assertions
}
```

多次调用使用 `respondSequence`。声明的 typed error 使用 `throw`，多次调用全部返回
typed error 时使用 `throwSequence`。stream effect 使用 `stream` 声明非空 event 序列：

```skiff
test "sequence and stream" effects {
  dependency.retry {
    respondSequence: [{ status: 503 }, { status: 200 }],
  },
  dependency.events {
    stream: [{ value: "first" }, { value: "second" }],
  },
  dependency.failure {
    throw: RequestFailure { code: "denied" },
  },
} {
  // assertions
}
```

`respond`、`respondSequence`、`throw`、`throwSequence` 与 `stream` 互斥；一个 target
必须且只能声明其中一个结果字段。序列使用 effect DSL 的 `[expr, ...]`，不是 Skiff
通用 array literal。

规则：

- compiler 必须解析精确 effect target，并静态检查 expect/respond/typed error/stream event；
- compiler 可以把 `effects` block 降低为 test-only hidden setup callable，但 setup 不是独立
  request，也不创建另一份 heap、activation 或 execution nonce；
- runner 对一个 case 只创建一次执行上下文，并在其中依次执行 setup 和 test body；setup
  产生的 response、error 和 stream event 必须立即按 linked target type plan
  materialize 到该 case 的 effect registry，不能把 heap value 作为跨执行共享对象保存；
- setup 成功后才执行 test body；setup 失败时 body 不执行；
- case finalization 是 runtime-owned teardown phase。无论 body 成功、assert 失败、throw、
  timeout 或 cancel，都必须检查未消费 double、销毁 registry 并释放 case 资源；
- 当前没有用户可写的 teardown 语法。未来若增加 teardown，它是同一 case execution 中位于
  body 之后、runtime finalization 之前的独立 phase，不改变现有 `effects` surface；
- effect declaration 只属于当前 case，case 完成后 registry 销毁；
- expected request subset 在真实 typed value materialization 后匹配；
- sequence 不能为空，未消费或超量调用必须产生明确测试失败；
- double 执行仍参与 effect conflict 和 capability policy；
- 不提供 JSON manifest compatibility loader；旧文件和旧 schema 必须迁移或删除。

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

test service 使用普通 artifact 格式，但 production publish/deploy workflow 必须根据
`kind: test` 拒绝它。Runtime 格式无需测试特例；测试权限在 compiler 名称解析阶段已经关闭。
