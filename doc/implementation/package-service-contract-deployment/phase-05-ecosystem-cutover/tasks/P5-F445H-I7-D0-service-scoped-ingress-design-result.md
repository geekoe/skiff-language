# P5-F445H-I7-D0 Service-scoped ingress design result

状态：

```text
PASS
D0_COMPLETE = YES
K_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
C_REVALIDATION_BLOCKED_ON_K_AND_CONSUMERS = YES
```

D0已经把HTTP Host从Skiff service route中移除，并把external ingress冻结为两阶段：
Router外部Host等规则注入可信service/version header；Router先选择唯一精确deployment，再只在该
deployment内按method/path选择handler。不同service共享相同method/path现在是合法设计，不再被裸全局
selector误判为collision。

D0只完成权威文档与后续代际合同，不声称production已经实现。K canonical
model/schema/identity/wire checkpoint现在可以启动；Relay/AIHub combined C必须等待K及其consumer迁移后
重验。

## 1. Parent chain and exact identities

直接task：

```text
P5-F445H-I7-D0-service-scoped-ingress-design.md
```

上游事实：

- I7R readiness与S1 Host/runtime/Router exact artifact receipt；
- C task与C1 provider checkpoint；
- Internals integration `48cfdf66c06bdc91781c67c84a5805bf4ba30bb4` /
  tree `79af10873f9bf7332259621c5e6bb15db8193466`，其中Relay与AIHub真实组合assembly暴露同
  `GET /v1/models` collision；
- D0/X1零worktree只读审计确认collision owner是
  `deployment/src/assembly/resolver.rs::insert_gateway_ingress`使用裸
  `BTreeMap<IngressSelector, GatewayIngressBinding>`；现行`IngressSelector`还在
  `artifact-model/src/deployment.rs`包含`host`。

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `cf43c08862d40e265fe660227aeff756b1dda406` / `d15431fd529ca24bfc12e32d42f84144551ae5a1` |
| task commit/tree | `9073ba5f1628cbc70a140b64811b6d71cc4ab2c2` / `c8d609b09f0d98a4ad7a48791b3129503d219bfb` |
| authority implementation commit/tree | `cbe05e8fa6cabf99c4eba944cca51a71940590a1` / `6e37d4412f2280be6359d5de429efbcb0246aa56` |
| branch | `codex/p5-f445h-i7-d0-ingress-docs` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-d0-ingress-docs` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree在Git handoff中报告；result不能自引用自己的commit identity。

## 2. Frozen contract

```text
external ingress
  Host / other platform mapping
  -> x-skiff-service + x-skiff-version

Skiff Router
  strict trusted headers
  -> exact ServiceDeploymentRef
  -> service-local IngressSelector
  -> GatewayEntryKey / GatewayEntryIdentity / exact handler

Router -> Runtime
  exact deployment + assembly generation + gateway entry
```

具体规则：

- HTTP Host只属于Router外部映射和HTTP业务metadata，不选择service、deployment或handler；
- Skiff不在Router内重做local ingress；直接带两个可信header调用Router是Skiff production receipt；
- HTTP selector是`(protocol, method, path)`，WebSocket upgrade selector是
  `(protocol, path)`；
- WebSocket upgrade固定精确deployment与generation；连接内JSON-RPC method只在该pin内解析；
- assembly ingress key是`(ServiceDeploymentRef, IngressSelector)`；
- 不同service可以共享相同selector，同一service内重复selector失败；
- 同一assembly中相同`serviceId + contractVersion`不得同时出现多个revision；
- 缺失、重复冲突、非法、未知或歧义service/version，以及跨deployment frame substitution全部
  fail closed；
- 旧Host route、裸全局ingress和旧wire是hard cut，不兼容读取。

## 3. Frozen generations

| Owner | Target generation |
| --- | --- |
| ServiceDeploymentInput | `skiff-service-deployment-input-v5` |
| ServiceDeployment schema | `skiff-service-deployment-v4` |
| DeploymentArtifact identity marker | `skiff-deployment-artifact-identity-v4` |
| DeploymentArtifact identity prefix | `skiff-deployment-artifact-v4:sha256` |
| RuntimeAssembly schema | `skiff-runtime-assembly-v3` |
| RuntimeAssembly identity marker | `skiff-runtime-assembly-identity-v3` |
| RuntimeAssembly identity prefix | `skiff-runtime-assembly-v3:sha256` |
| Router↔Runtime frame | `skiff-runtime-frame-v2` |

GatewayEntryIdentity/GatewayEntry保持v2；ServiceContract/ServiceProtocol、Package artifact/build/local
ABI/schema与WebSocketEntryId不变。

## 4. Actual write set

Architecture and reference：

```text
doc/architecture/package-service-contract-deployment.md
doc/architecture/runtime-deployment-topology.md
doc/architecture/gateway-runtime-adapter-boundary.md
doc/reference/service-yml.md
doc/reference/runtime.md
router/README.md
```

Phase and historical evidence：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/
  phase-overview.md
  phase-plan.md
  tasks/P5-T03-router-active-assembly-cutover.md
  tasks/P5-F03B-router-integration-repair.md
  tasks/P5-F03B-router-integration-repair-result.md
  tasks/P5-F445H-I7-D0-service-scoped-ingress-design.md
  tasks/P5-F445H-I7-D0-service-scoped-ingress-design-result.md
```

没有production、schema/identity constants、fixtures、tests、Cargo、Internals、official packages或外部
状态写入。

## 5. Historical evidence invalidation

| Evidence | D0 classification |
| --- | --- |
| P5-T03 Host/global-selector完成态、Host collision与header negative | `INVALID`；替换为service-scoped header routing正负例 |
| P5-T03 activation/replica/generation事实 | `HISTORICAL_ONLY`；新assembly/frame代际后做受影响回归 |
| P5-F03B route/header/global-map及总体64-test ledger | `INVALID_FOR_NEW_INGRESS` |
| P5-F03B store/participant/generation pin/drain | `HISTORICAL_ONLY`；新代际后回归，不被D0直接否定 |
| S0/S1 exact DeploymentArtifact v3 / RuntimeAssembly v2 / frame v1 tuple | `INVALIDATED_BY_K` |
| S1 canonical source→artifact→Host/Router receipt结构 | `REUSABLE_TEST_STRUCTURE`；不能沿用旧identity结果 |
| C1/C2 Relay/AIHub局部provider/caller实现证据 | `UNAFFECTED_LOCALLY` |
| Relay+AIHub combined assembly与isolated verdict | `BLOCKED`；等待K与consumer迁移后重验 |

## 6. Documentation evidence

| 层级 | 命令 / 检查 | 结果 | 覆盖 |
| --- | --- | --- | --- |
| baseline identity | integration status + `rev-parse HEAD HEAD^{tree}` | PASS；clean exact `cf43c088` / `d15431fd` | 零worktree预检输入 |
| generation preflight | production constant `git grep` | PASS；current v4/v3/v3/v2/v1分别单步升级到v5/v4/v4/v3/v2 | 版本无冲突 |
| write scope | `git diff --name-only <baseline>` | PASS；仅上述13个文档 | no production/test writes |
| whitespace | `git diff --check <baseline>` | PASS | trailing whitespace / conflict marker |
| Markdown fences | 对全部改动Markdown统计行首```并检查偶数 | PASS | fenced block配对 |
| positive contract search | service headers、scoped key、精确deployment、v5/v4/v3/v2搜索 | PASS | 权威/reference/phase一致 |
| stale semantics search | `protocol, host`、`globalIngress`、Host/header旧oracle | PASS；只剩D0 preflight事实或T03/F03B明确标记的撤回历史 | 旧语义不再具有权威性 |

文档任务未运行build/test/live/stable/network/Mongo/OAuth/browser；这些都不属于D0证据。

## 7. Downstream handoff

K现在以本result和`doc/architecture/package-service-contract-deployment.md`为唯一公共契约输入，先实现：

1. canonical model/schema/identity/version hard cut；
2. scoped assembly key与同坐标多revision拒绝；
3. runtime frame v2中的精确deployment；
4. old Host field/wire strict rejection。

K稳定后可并行扇出compiler/authoring、assembly/loader/linker与Router/Runtime/Host-wire consumers，最后统一
刷新fixtures/goldens并运行真实join：

- Relay与AIHub同时声明`GET /v1/models`；
- 同一Router Host/port，不同service/version header精确选中不同deployment；
- runtime frame携带并执行同一个exact deployment；
- 同service重复route、缺失/非法header、同坐标多revision、旧Host wire与跨deployment替换全部失败。

```text
D0_COMPLETE = YES
K_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
