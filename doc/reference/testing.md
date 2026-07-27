# Skiff Testing Reference

本文负责测试源码、测试发现、runtime 执行语义、package 测试、测试service配置profile和
production artifact 边界。本文不负责具体 CLI flag、测试进程编排、secret文件分发或runner的实现细节。

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
- test service 的 config profile固定为`skiff-test`，来自`config.skiff-test.yml`；
- 本机或部署时的私密覆盖使用同profile的`config.skiff-test.secret.yml`，该文件不得提交；
- 同一个 test service 的 cases 共享配置和 dependency graph，但每个 case 仍有独立 state
  namespace、heap、effect registry 和 execution nonce；
- 需要不同配置时使用另一个test service，不提供per-case config override，也不允许调用方切换
  test service config profile。

测试service配置profile与runtime目标environment是两个概念：

- `skiff-test`固定选择测试service的配置和secret overlay；
- live runner的target environment标识外部Router/Runtime中的activation generation，可能是`dev`或
  其它部署环境；
- target environment不得反向选择`config.<environment>.yml`。普通隔离测试中两者通常都叫
  `skiff-test`，不能因此在实现里合并两个owner。

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
- live只改变runtime target ownership和effect policy；test service仍固定读取
  `config.skiff-test.yml`及可选`config.skiff-test.secret.yml`。
- activation URL、ingress URL、artifact root、target environment与expected generation是每次运行的
  显式target参数，不属于test service config，也不能写进secret overlay。
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

单次调用使用 `respond`、`throw` 或 `stream`。多次调用统一使用 `sequence`，每一步可以
声明自己的 request subset 和一种结果：

```skiff
test "unary and stream sequences" effects {
  dependency.request {
    expect: {
      method: "POST",
    },
    sequence: [
      {
        expect: { url: "https://example.test/first" },
        respond: { status: 503 },
      },
      {
        expect: { url: "https://example.test/second" },
        throw: RequestFailure { code: "denied" },
      },
      {
        expect: { url: "https://example.test/third" },
        respond: { status: 200 },
      },
    ],
  },
  dependency.events {
    sequence: [
      {
        expect: { url: "https://example.test/events/first" },
        stream: [{ value: "first" }, { value: "second" }],
      },
      {
        expect: { url: "https://example.test/events/second" },
        throw: RequestFailure { code: "disconnected" },
      },
    ],
  },
  dependency.failure {
    throw: RequestFailure { code: "denied" },
  },
} {
  // assertions
}
```

`respond`、`throw`、`stream` 与 `sequence` 互斥；一个 target 必须且只能声明其中一个
结果字段。`sequence` 必须非空，每一步可以声明一个可选 `expect`，并且必须且只能声明
`respond`、`throw` 或 `stream` 之一，但只能使用该 target 签名允许的结果。普通 unary
target 的步骤只能是 `respond` 或签名声明的 typed `throw`；直接返回 `Stream<T>` 的
target 只能是 `stream` 或签名声明的 typed `throw`。不把 unary response 隐式解释成
stream，也不把 `respond` 隐式解释成 `Stream<T>` 的单个 event 或完整 stream。target
顶层 `expect` 是每一步都必须满足的公共 request subset；step `expect` 是该次调用额外
必须满足的 subset。两者分别匹配并取逻辑 AND，不做 object merge 或覆盖。序列和 stream
event 表使用 effect DSL 的 `[item, ...]`，不是 Skiff 通用 array literal。顶层
`expect` 表达式在 setup 中只求值一次；Runtime 保存其 wire 快照，并对 sequence 的每一步
复用同一快照。

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
- 同一个精确 linked target 在一个 case 中只能声明一次；不同 alias 如果解析到同一个
  Package callable 或 service operation，也必须拒绝并要求写成一个显式 `sequence`；
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

Runner flag只控制runtime target ownership、target environment/generation和effect policy，不改变
测试源码语义，不选择test service config profile，也不把`defaultRun false`文件加入目录默认发现。
非live与live都不切换到compiler VM。

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
