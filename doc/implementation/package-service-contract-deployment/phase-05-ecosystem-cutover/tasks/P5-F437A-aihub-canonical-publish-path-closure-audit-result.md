# P5-F437A AIHub canonical publish path closure audit result

状态：`AUDIT_COMPLETE / CANONICAL_PATH_NOT_CLOSED / TASK_SCOPE_NOT_EXPANDED`。

结论分成四层：

1. F436A 的 `skiff.run/http-session@1.0.0` pointer 首错是 **canonical fixture omission**，
   不是当前 official package source defect。当前 exact `skiff-packages` 输入中的
   `http-session` 与 `track` 均可从 fresh isolated store 成功 publish，并生成
   `PackageArtifact` record/pointer。
2. 把`http-session`与`track`两个官方包的包根目录按依赖顺序补入同一个isolated store后，Codex Relay、AIHub与
   Account 均成功生成 Package/Contract/Deployment receipts；AIHub 已完整越过 F434A–F436A
   的 generic return、interface receiver 与 official pointer 三层旧首错。
3. 完整 service publish 继续在 Agine 暴露当前 production blocker：
   `agine/service/service.yml` 仍使用已被当前 Skiff schema 明确拒绝的
   `websocket.routes[].operation`。同一 root 还存在被该首错遮挡、可由当前 typed requirement
   静态确定的 config binding 错位。
4. 对已经成功 publish 的 Codex Relay、AIHub、Account 做 maximal-success assembly probe，
   assembly 正确拒绝 Codex Relay 与 AIHub 共同声明的
   `HTTP host="*" GET /v1/models`。因此即使 Agine repair 完成，当前 full assembly 仍不会闭合。

本 leaf 没有承接任何 repair，也没有运行 HTTP stream combined、service E2E、runtime、provider、
stable 或 live。

## 1. Exact inputs、写集与范围判定

| repo | audit input commit | tree | audit worktree HEAD 说明 |
| --- | --- | --- | --- |
| Skiff production | `1b4fb5b81049ef310e539f2888a96237895012d6` | `65b6edb922448cd32174b673bf41717d9caa4f08` | 起始 HEAD `681aa3db2f559e8b20ee8812f6803a043a5ee239` 只比 production input 多本任务文件 |
| Internals | `066b5135a8e06f87acfd614e408e05b35453f4eb` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` | 精确命中 |
| skiff-packages | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` | 精确命中 |

三个 worktree 起始均 clean。Internals、skiff-packages、Skiff production/test/fixture 均未修改；
唯一写入是本文。

`TASK_SCOPE_EXPANDED = NO`。Phase 05 已在
`P5-T09E-internals-final-assembly.md` 和 cross-repo 命令中冻结显式
`SKIFF_ROOT` + `SKIFF_PACKAGES_ROOT` 输入形态。当前问题是 executable workflow 尚未消费后者，
不是需要本 leaf 新选一个 public fallback。repair 不应增加 sibling guessing、main-worktree
artifact fallback 或兼容路径。

## 2. 完整 dependency closure

下表只列四个 service roots 的可达 publish closure。`std` 是 bootstrap owner 生成的 platform
Package；其余每个依赖均来自当前 `package.yml` 的 exact version/alias。

| publish root | coordinate | direct package dependencies | direct service dependencies |
| --- | --- | --- | --- |
| Skiff `std` | `skiff.run/std@1.0.0` | 无 | 无 |
| Internals `packages/llm-api` | `agine.ai/llm-api@0.1.0` | 无 | 无 |
| Internals `packages/llm-providers` | `agine.ai/llm-providers@0.1.0` | `llmApi -> agine.ai/llm-api@0.1.0` | 无 |
| Internals `packages/agent` | `agine.ai/agent@0.1.0` | `llmApi -> agine.ai/llm-api@0.1.0` | 无 |
| skiff-packages `http-session` | `skiff.run/http-session@1.0.0` | `std` 由 compiled requirement 注入 | 无 |
| skiff-packages `track` | `skiff.run/track@1.0.0` | `httpSession -> skiff.run/http-session@1.0.0`；`std` 由 compiled requirement 注入 | 无 |
| Internals `codex-relay/service` | `agine.ai/codex-relay@0.1.0` | `llmProviders -> agine.ai/llm-providers@0.1.0`；`llmApi -> agine.ai/llm-api@0.1.0` | 无 |
| Internals `aihub/service` | `agine.ai/aihub@0.1.0` | `llmApi -> agine.ai/llm-api@0.1.0`；`llmProviders -> agine.ai/llm-providers@0.1.0` | `codexRelay -> agine.ai/codex-relay@0.1.0` |
| Internals `agine/service` | `agine.ai/api@0.1.0` | `httpSession -> skiff.run/http-session@1.0.0`；`track -> skiff.run/track@1.0.0`；`agent -> agine.ai/agent@0.1.0`；`llmApi -> agine.ai/llm-api@0.1.0`；`llmProviders -> agine.ai/llm-providers@0.1.0` | `aihub -> agine.ai/aihub@0.1.0` |
| Internals `skiff-platform/account` | `skiff.run/account@0.1.0` | `httpSession -> skiff.run/http-session@1.0.0` | 无 |

有向 DAG（箭头为 prerequisite → consumer）：

```text
skiff.run/std@1.0.0
  └─► every compiled package that records the implicit std requirement

agine.ai/llm-api@0.1.0
  ├─► agine.ai/llm-providers@0.1.0
  ├─► agine.ai/agent@0.1.0
  ├─► agine.ai/codex-relay@0.1.0
  ├─► agine.ai/aihub@0.1.0
  └─► agine.ai/api@0.1.0

agine.ai/llm-providers@0.1.0
  ├─► agine.ai/codex-relay@0.1.0
  ├─► agine.ai/aihub@0.1.0
  └─► agine.ai/api@0.1.0

agine.ai/agent@0.1.0 ─────────────────────────► agine.ai/api@0.1.0

skiff.run/http-session@1.0.0
  ├─► skiff.run/track@1.0.0 ──────────────────► agine.ai/api@0.1.0
  ├─► agine.ai/api@0.1.0
  └─► skiff.run/account@0.1.0

agine.ai/codex-relay@0.1.0
  └─► agine.ai/aihub@0.1.0
        └─► agine.ai/api@0.1.0

{codex-relay, aihub, agine, account} deployments ─► RuntimeAssembly
```

依赖偏序本身有独立分支，因此数学上不只有一个 linear extension。为给本 workflow 一个唯一且不改变
现有 roots 相对顺序的 canonical linearization，本审计采用明确 tie-break：先保留当前 package
array，再把新发现的 official roots 按自身依赖顺序追加到 package stage，最后保留当前 service
array。所得顺序是：

```text
1. skiff/std
2. packages/llm-api
3. packages/llm-providers
4. packages/agent
5. skiff-packages/http-session
6. skiff-packages/track
7. codex-relay/service
8. aihub/service
9. agine/service
10. skiff-platform/account
11. assembly build from the four service deployment receipts
```

这同时保持 workflow 的“全部 dependency packages 在全部 service packages 之前”阶段约束。
独立节点的其它 topo linearization 不改变 artifact 语义；repair 只需让 deterministic tie-break
机械稳定，不需要为此新增 public behavior。

### 2.1 当前 fixture 差异

| 分类 | root | 结论 |
| --- | --- | --- |
| missing | exact skiff-packages checkout 的 `http-session` | Agine 与 Account 的直接 prerequisite；Track 的 prerequisite |
| missing | exact skiff-packages checkout 的 `track` | Agine 的直接 prerequisite |
| duplicate | 无 | 当前八个显式 roots 没有 path duplicate |
| inversion among present roots | 无 | `llm-api -> {llm-providers,agent}` 与 `codex-relay -> aihub -> agine` 的现有相对顺序正确 |
| unsatisfied scheduled consumer | `agine/service` | 被排在缺失的 `http-session`、`track` 之后无从成立；因此先报 `http-session` pointer |
| unsatisfied scheduled consumer | `skiff-platform/account` | 同样缺 `http-session`；因位于 Agine 之后而被首错遮挡 |

## 3. 为什么 AIHub target 会进入 Agine/Account blocker

这不是 resolver 把 AIHub 误认成 Agine，而是 target 参数从未参与 fixture selection：

1. `scripts/check-isolated-service-graph.mjs:12-16` 只读取 target，并用 `serviceNode(targetId)`
   校验它是四个已知 id 之一。
2. 第 17 行不传 target，直接调用 `runCanonicalFixtureWorkflow({ internalsRoot, skiffRoot })`。
3. `prepare-canonical-assembly.mjs:15-28` 的 `canonicalFixture` 固定包含全部四个 service roots；
   `:57-65` 总是把这份 full fixture 交给执行器。
4. `:76-98` 先 publish 全部 package list，再按
   Codex Relay → AIHub → Agine → Account 顺序 publish 全部 services。因此 AIHub 自己成功后，
   下一个 Agine 才消费 `http-session`/`track`；Account 的同类依赖排在更后面。
5. receipts 由 `:76` 的三数组累积；每个 service 当场要求 Package/Contract/Deployment 三份
   receipt，只有全部 roots 成功后才在 `:99-118` 写四-root assembly 并把 receipts 交给 callback。
   任一中途失败都没有 target-specific partial success verdict。
6. assembly owner 是 Skiff `compiler/driver/authoring.rs`：它从 authoring root 的 deployment refs
   读取 typed records，再沿 `serviceSelectors` pointer 闭包 provider deployments；Internals
   workflow 只负责把四个已生成 deployment refs 写入临时 `assembly.yml`。

因此：

```text
aihub/service npm type-check
  -> validate target id only
  -> run full canonical fixture
  -> publish AIHub successfully
  -> enter Agine
  -> missing http-session pointer in the current incomplete fixture
  -> Account and assembly are masked
```

## 4. Official roots 如何进入 linked-worktree workflow

### 4.1 当前 owner

| concern | current executable owner | 当前事实 |
| --- | --- | --- |
| Internals checkout | `check-isolated-service-graph.mjs` 由自身 script path 推导 `internalsRoot` | 使用调用脚本所在 exact worktree；不是 sibling guess |
| Skiff checkout | `SKIFF_ROOT`，随后传给 `skiffCli` 与 std bootstrap `--platform-source-root` | 当前仍有 `../skiff` fallback；linked canonical invocation 应要求显式值 |
| dependency root list | Internals `prepare-canonical-assembly.mjs::canonicalFixture` | 只认识 `<skiff>/std` 和 Internals 相对 roots |
| official checkout | 无 executable owner | `SKIFF_PACKAGES_ROOT` 完全未读取；即使设为不存在路径，`--list` 仍 exit 0 且输出不含 official roots |
| Package authoring CLI | Skiff `scripts/lib/package-service-authoring.mjs:210-227,231-307` | positional absolute root + explicit `--artifact-root`；compiler 同时收到 exact `--platform-source-root` |
| dependency resolution | Skiff `compiler/driver/authoring.rs:337-455` | 只按 manifest 的 exact id/version 从当前 artifact root 读 typed pointer/record；不发现 sibling source |
| assembly resolution | Skiff `compiler/driver/authoring.rs:610-643` | 从 typed deployment refs/pointers 闭合 service providers |

### 4.2 Required reproducible input shape

linked-worktree canonical workflow 应机械接收并验证：

```text
internalsRoot = executable script's own checkout root
SKIFF_ROOT = required absolute exact Skiff checkout
SKIFF_PACKAGES_ROOT = required absolute exact skiff-packages checkout
artifactRoot = one newly-created child of the owned isolation root
```

随后 root discovery 必须从这三份输入中的 manifests 计算 closure，并把
`$SKIFF_PACKAGES_ROOT/http-session`、`$SKIFF_PACKAGES_ROOT/track` 作为普通 positional roots
传给现有 `skiff package publish`。workflow receipt 应记录三仓 absolute root、commit、tree 与每个
coordinate→source root 映射，使 wrong checkout 可在 authoring 前失败。

禁止：

- 读取 stable store、`.skiff-instance`、主 worktree artifact 或 package pointer；
- 创建 `.skiff-packages` source symlink；
- 由 Internals 路径猜 `../skiff-packages`；
- 在缺 env 时静默退回 sibling/main/stable；
- 缺 root 时由 compiler 或 source resolver 隐式补包。

## 5. Bounded crossing probe

唯一临时根：

```text
/private/tmp/p5-f437a-aihub-publish-audit.kMbYqW
```

其中包含隔离 `artifacts`、`cargo-target`、`npm-cache`、`tmp` 与 logs；所有 publish 均使用三个
exact worktree source roots和同一个 fresh artifact root。probe 只执行 std bootstrap、按第 2 节
顺序的 Package/Service authoring，以及一次 maximal-success assembly build。

### 5.1 Crossing ledger

| stage | root | result | receipt / diagnostic | blocked or exposed |
| --- | --- | --- | --- | --- |
| bootstrap | `skiff.run/std@1.0.0` | PASS | build `skiff-package-build-v10:sha256:3604e31ffac0e1a12432e213fb895a51fef18355b365e1d897147a6c43924695`；pointer present | 解锁全部 source compilation |
| package publish | `agine.ai/llm-api@0.1.0` | PASS | Package record + pointer | 解锁 internal dependents |
| package publish | `agine.ai/llm-providers@0.1.0` | PASS | Package record + pointer | 解锁 Relay/AIHub/Agine |
| package publish | `agine.ai/agent@0.1.0` | PASS | Package record + pointer | 解锁 Agine package dependency |
| package publish | `skiff.run/http-session@1.0.0` | PASS | build `skiff-package-build-v10:sha256:ea2d1ac4529a9a005e74b01b72672ab500bbd12dba9dadee0d258b3ebfeb5398`；local ABI `skiff-package-local-abi-v7:sha256:2b0de422e0bd5e496492bd60880228e90a11053edcae3ae497ad2852e3d0e28a`; pointer present | 证明 F264 历史 API 缺口已修；解锁 Track/Account/Agine |
| package publish | `skiff.run/track@1.0.0` | PASS | build `skiff-package-build-v10:sha256:60080c9361c451ec40203b59a4adc7db7c8dd6ecf16cd52e85a108e437eb5d4b`；local ABI `skiff-package-local-abi-v7:sha256:c853b99fd075fdfdaee5c99fa64b945deeb764a74c2515d0bce0d9d41beb50cf`; pointer present | 解锁 Agine direct dependency |
| service package publish | `agine.ai/codex-relay@0.1.0` | PASS | Package + ServiceContract + ServiceDeployment records/pointers | AIHub provider ready |
| service package publish | `agine.ai/aihub@0.1.0` | PASS | build `skiff-package-build-v10:sha256:17624dcc945815d35a320cdea10bf5b5859c91db6799cfbf473a66d035b60ed8`; protocol `skiff-service-protocol-v5:sha256:d8ef2bc6315089561746a3922c570663382cd6609302e25cf4f2a4ec9d54b4e7`; all three typed receipts/pointers | F434A/F435A/F436A old blockers all crossed |
| service package publish | `agine.ai/api@0.1.0` | **FAIL** | `failed to parse .../agine/service/service.yml: websocket: unknown field 'routes', expected one of 'host', 'path', 'connect' at line 293 column 3` | blocks Agine package compilation, API receipt, Contract, Deployment and full assembly |
| service package publish | `skiff.run/account@0.1.0` | PASS (independent continuation) | Package + ServiceContract + ServiceDeployment records/pointers；http-session config/state bindings validated | 证明 Account 不是 official source blocker |
| assembly build | successful roots `{codex-relay, aihub, account}` | **FAIL_INDEPENDENT_BLOCKER** | `gateway ingress selector ... Http, host: "*", method: GET, path: "/v1/models" ... declared by both` AIHub and Codex Relay | full assembly 也会被同一 collision 阻止 |

Agine failure 是 source-control parse 阶段，发生在
`compiler/driver/authoring.rs:131-140` 的 `read_service_package_root`，尚未进入 Agine package source
compile。当前 Skiff authoring schema 在
`artifact-model/src/ecosystem_authoring.rs:124-136,215-235` 把 WebSocket 定义为一个 strict
`{host,path,connect}` entry，并以 `deny_unknown_fields` 明确拒绝 `routes`。

partial assembly 没有改 source 或伪造 Agine pass；它只消费 probe 中真实成功生成的三个 deployment
receipts，用来枚举被 Agine 首错遮挡的 assembly owner blocker。静态枚举四个 service manifests 的
所有 94 个 HTTP entries，只发现这一组重复 selector：

```text
Codex Relay v1ModelsGet: HTTP * GET /v1/models
AIHub      v1ModelsGet: HTTP * GET /v1/models
```

### 5.2 被 Agine parse 首错遮挡的 config blocker

fresh `http-session` artifact 的 typed runtime requirements 是：

```text
config:
  cookieName: required string
  maxAgeSeconds: required number
  cookieDomain: optional string
  publicPaths: optional Json
state:
  http-session-store: database
```

`agine/service/config.dev.yml:24-27` 当前只提供一个 top-level object key `httpSession`，其下嵌套
`cookieName`/`maxAgeSeconds`。deployment config binder 不按 package alias namespace requirement；
它在 `deployment/src/projection/requirements/config.rs:9-47` 按 exact path 比较。

静态 focused comparison 得到：

```text
required top-level paths: [cookieName, maxAgeSeconds]
actual top-level paths:   [httpSession]
missing:                  [cookieName, maxAgeSeconds]
extra:                    [httpSession]
predicted first typed diagnostic after the WebSocket parse repair:
  missing config binding cookieName
```

这是当前真实、被首错遮挡的 Internals profile defect。Agine profile 已正确列出
`http-session-store` 与 `track-store` state bindings；state 不是该 blocker。

## 6. Official package recheck

### 6.1 `http-session`

| surface | current verdict |
| --- | --- |
| Package API closure | PASS。`http-session/api.yml` 已显式发布 `HttpSessionSource: session.HttpSessionSource`；fresh publish 接受 API closure |
| F264 历史结论 | 已修。`HttpSessionSource` 的当前行来自 `609551f0`，该 commit 是 exact packages input 的 ancestor |
| state declaration | PASS。`package.yml` 声明 `http-session-store: database`；state repair `337e3fae` 也是当前 input ancestor |
| config requirements | typed artifact 精确记录上节四项；Account profile 的 unprefixed `cookieName/maxAgeSeconds` 动态生成 Deployment 成功 |
| source typing | PASS。`session.skiff` 在 current Skiff compiler 上完整 publish |
| publication pointer | isolated probe 生成 exact `skiff.run/http-session@1.0.0` pointer；当前 canonical fixture 未生成它只是因为 root 未枚举 |
| consumers | Agine 与 Account 均声明 version `1.0.0`, alias `httpSession`，与 official manifest 完全匹配 |

### 6.2 `track`

| surface | current verdict |
| --- | --- |
| Package API closure | PASS。公开 surface 只有 `record: track.record` |
| dependency | exact `httpSession -> skiff.run/http-session@1.0.0`；probe 在 http-session 之后成功 publish |
| state declaration | PASS。`track-store: database` 进入 artifact runtime requirements |
| source typing | PASS。包括 `httpSession.HttpSession` type 与 `httpSession/...` callable 使用 |
| publication pointer | isolated probe 生成 exact `skiff.run/track@1.0.0` pointer；fixture omission 是唯一缺失原因 |
| consumer | Agine 声明 version `1.0.0`, alias `track`，完全匹配；profile 已绑定 `track-store` |

因此 official source repair 集合为空：

```text
OFFICIAL_SOURCE_REPAIRS = {}
PURE_FIXTURE_OMISSIONS = {http-session root, track root}
CURRENT_OFFICIAL_POINTER_BLOCKERS = {}
```

## 7. Tests、receipts 与最小负例

实际运行以下 current tests，`19 passed / 0 failed`：

```text
scripts/isolated-service-graph.test.mjs
scripts/prepare-canonical-assembly.test.mjs
scripts/test-isolated-service.test.mjs
agine/service/service-api-receipt.test.mjs
```

它们全绿但未发现本次三个 production 问题，原因如下。

| current coverage | gap | minimum negative strengthening |
| --- | --- | --- |
| `canonicalFixtureInputs` 的 expected array 精确锁定当前四 package + 四 service roots | expected 自身缺 official roots，测试把 omission 当正确值 | 从四个 service manifests 计算 dependency coordinates；删掉 `http-session` 或 `track` root 时在执行任何 authoring 前报 missing coordinate/root |
| `assertCanonicalInputs` 拒绝 empty、path duplicate、package/service 同 root | 不验证 manifest closure 或 topo order | 将 `track` 排在 `http-session` 前、`aihub` 排在 Relay 前分别 fail；完整正确 order pass |
| `assertCompleteBuildReceipts` 只检查 expected 中是否 missing | expected 可不完整；`Set` 还会掩盖 duplicate receipt，并忽略 extra | receipt coordinates 必须与 computed closure 做 exact multiset equality；分别注入 missing、duplicate、extra receipt |
| service receipt 当场要求 Package/Contract/Deployment 三字段 | 只有全 workflow 成功后才检查 root completeness；失败时不能描述 partial writes | 模拟某 service 只有 Package/Contract 无 Deployment，并断言 workflow 返回 stage/root/partial-record ledger，且不进入 assembly |
| `SKIFF_ROOT` 路由被 package scripts 静态检查 | 不要求 env，不核对 checkout；完全不读取 `SKIFF_PACKAGES_ROOT` | missing env、non-absolute root、root outside supplied checkout、manifest id/version mismatch、wrong commit/tree provenance 分别 fail closed |
| tests 拒绝 `--packages-dir` / `--service-artifact-root` | executable custom workflow 分支没有对 root provenance 做等价检查 | 对实际 `executeCanonicalFixture` invocation 断言所有 roots 位于三份显式 checkout、artifact root 位于 isolation root、无 stable/main path |
| Agine receipt test 第 89-92 行主动要求 legacy `websocket.routes.operation` | 与当前 Skiff strict schema 相反，形成 false-positive | 改为 canonical singleton WebSocket shape 正例；`routes`、`operation`、multi-entry map 各自负例 |
| `assertUniqueAssemblySelectors` 只测手写重复 Host 数组 | 不消费生成 deployments，没发现 wildcard route collision | 用 Relay + AIHub generated ingress 做 assembly negative；赋予冻结的 distinct Hosts 后 pass |

另外，receipt 当前没有三仓 commit/tree provenance。即使 root path 指向错误 checkout，只要 coordinate
相同，现有 workflow receipt 也无法让 caller 证明 exact source input。

## 8. Remaining blocker matrix

| ID | classification | repo / owner | evidence | downstream |
| --- | --- | --- | --- | --- |
| R0 | `CANONICAL_ROOT_DISCOVERY_OMISSION` | Internals `scripts/prepare-canonical-assembly.mjs` + shared workflow tests | executable list 缺 `http-session`/`track`，忽略 `SKIFF_PACKAGES_ROOT` | 原 canonical type-check 永远先停在 missing pointer |
| R1 | `AGINE_LEGACY_WEBSOCKET_AUTHORING` | Internals `agine/service/service.yml` + receipt/source owner | dynamic parse failure at `service.yml:293`; current receipt test主动锁旧形态 | Agine package/source/API/Contract/Deployment/full assembly |
| R2 | `AGINE_CONFIG_REQUIREMENT_BINDING_MISMATCH` | Internals `agine/service/config.dev.yml` | typed official requirement vs current profile static focused comparison | Agine Deployment；在 R1 后成为预计首错 |
| R3 | `CROSS_SERVICE_INGRESS_SELECTOR_COLLISION` | Internals Codex Relay/AIHub ingress authoring + receipt tests | dynamic assembly rejection for `HTTP * GET /v1/models` | any assembly containing AIHub and its Relay provider |
| R4 | `MASKED_UNKNOWN_AFTER_AGINE_PARSE` | future cheap combined owner | Agine source compile 未到达，不能声明其余 source/contract/deployment PASS | 只能在 R1/R2 修复后由同一便宜 probe收敛 |

不是 remaining blocker：

- F434A generic concrete-return typing：本 probe 的 AIHub publish PASS。
- F436A interface self receivers：本 probe 的 AIHub publish PASS。
- F264 `HttpSessionSource` API closure：当前 official publish PASS。
- `http-session`/`track` source、state 或 version/alias：当前均 PASS。

## 9. Batch repair DAG

```text
R0 shared workflow/root-discovery checkpoint (Internals scripts owner)
  - require explicit SKIFF_ROOT + SKIFF_PACKAGES_ROOT for linked canonical work
  - validate/record three exact checkout roots + commit/tree
  - compute manifest closure and deterministic topo order
  - add http-session then track
  - exact receipt multiset + stage/root/partial ledger
  - add omission/order/wrong-checkout/partial-receipt negatives
  |
  +─► O0 official package revalidation only
  |     - publish exact http-session then track
  |     - assert API/config/state/pointer/consumer coordinates
  |     - no skiff-packages source repair currently authorized or required
  |
  +─► I1 Agine authoring/profile repair (Internals Agine owner)
  |     - replace rejected websocket.routes.operation authoring with current strict owner
  |     - update the receipt test that currently requires the rejected shape
  |     - bind cookieName/maxAgeSeconds at their exact typed config paths
  |     - preserve http-session-store/track-store bindings
  |
  +─► I2 explicit ingress Host closure (Internals service authoring owner)
        - assign frozen Hosts: codex-relay.localhost, aihub.localhost,
          agine.localhost, account.skiff.localhost
        - minimally prove Relay and AIHub /v1/models selectors differ
        - generated-deployment/assembly collision negative

R0 + O0 + I1 + I2
  └─► C0 cheap isolated publish/assembly integration
        - same ten-root order from §2
        - all Package/Contract/Deployment receipts exact
        - one four-service RuntimeAssembly receipt
        - no HTTP/runtime/live
        - enumerate any new Agine source/contract/deployment blocker in one pass

C0 PASS only
  └─► C1 AIHub full combined rerun
        - existing full canonical type-check/test/isolated HTTP stream combined owner
        - no stable/live provider unless separately authorized
```

I1 与 I2 都可能触碰 `agine/service/service.yml`，实现时必须由同一 owner 串行落地或先冻结不重叠
hunk；不得让两个并行 repair worktree 互相覆盖。本文不承接这些节点。

## 10. Isolation、cleanup 与禁止项

- 唯一临时 root 初始约 2.0 GiB，包含全部可重建 Cargo/artifact/log 状态。
- 交付前已递归删除
  `/private/tmp/p5-f437a-aihub-publish-audit.kMbYqW`，并验证 path absent。
- 未读取或修改 stable artifact store、watch registry、router、runtime、telemetry、MongoDB 或固定端口。
- 未创建 source symlink，未读取主 worktree artifact，未调用 reload。
- 未运行 HTTP stream combined、service E2E、Agine chat smoke、provider、stable 或 live。
- 未 merge、rebase 或 push。
- result commit/tree 与三个最终 clean 状态由交付消息记录。
