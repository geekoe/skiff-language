# Phase 05：Ecosystem Cutover 实现计划

状态：active；P5-R01 已在 `c168b1dc` / tree `961998ac` PASS。T02/T03/T04已合流；T05 candidate
`f8ad689`接收审查FAIL，D04已冻结repair设计、F04在途；
R02预审在`b47ddf7`发现T03/T04真实wire/startup/request/pin/storage双owner断链。F03A已合流，R02A在
`a7566bb`首次FAIL后完成D03/F03A1；第二次在`5715497`因raw Unicode/opaque number/default normalization
不一致而FAIL。验收熔断D06/F03A2已在`4df6c04`通过R02A第三次窄验收。F04探针发现正常authoring不能
产生可观测WS pin，D05已冻结typed unified WS ABI；F04真实consumer又由callable-effects过度fail-close阻断，
D07已冻结F06 compiler repair；R06首次因exact callee的Field wrapper遗漏而FAIL，F06A已在`fbf634d`通过
R06窄复验；D08/F07 exact native semantics已在`bd13867`通过R07。F04的Host证据与F03C依赖形成环，D09已冻结
F08/R08前置Host seam修复已在`c5ec7ea`通过；F04原Host probe又发现environment丢失与helper fixture未执行，
D10/F04A已形成`7f36810`implementation checkpoint。D11/F09已在`ff7a4df`通过R10并接通Router control wire；
D12/F10已在`efb2bbbe`通过R11并恢复Runtime committed bootstrap。真实probe现已ready并进入std，F04B只修复
source-suite缺少canonical `--bin`的直接caller并已合流`c06e115`；完整std现暴露crypto/time exact effects覆盖遗漏，
D13/F11已在`2d74b2c`通过R12。真实std request继续暴露双端canonical unary consumer未接线，D14已冻结并行
F12/F13并已在`02b97ff`通过combined R13；std 11/11后Host assembly暴露linked FileIR locator误比较，D15已冻结
F14已在`bcbdc2c`通过R14。原样gate现仅暴露activation receipt→healthy registration毫秒级空窗，D16已冻结
F15 readiness barrier首次被R15拒绝，D17/F15A在`e3a0d78`形成实现checkpoint；旧计划声称的R15 PASS没有独立ledger，
D20后的独立复验确认readiness语义关闭，但在`e786671`因F16C新增8参helper的Clippy回归FAIL。F04原样gate曾在`40ed693`
暴露共享Cargo target复用编译期platform绝对路径；D18已以双worktree镜像实验冻结F16A shared context及F16B/F16C
transport扇出。因同一F04真实入口已连续暴露多个跨层blocker，收敛熔断已触发；D20在`e786671`以六个新只读
审计/预读节点闭合14跳矩阵，并把余项批量冻结为F18A–F18I九个互斥写owner。首次I16与R15B PASS后，R18A发现
authoring guard前仍创建store；F18J已作为同owner窄后继修复合流。旧I16/R15B随候选失效，新candidate只重跑一次
cheap combined，并通过全新窄验收、H18与R16；G16第一次full-mode调用在Host前被root-sensitive Cargo dep-info误判，
且失败取证/Host计数、final PASS解析与inner isolated workspace ownership仍有缺口。D25三路只读审计冻结的F19A/B已
作为两个clean commit合流；下一步重建v4 combined/窄证据，再执行本周期第二次且原则上最后一次full-mode gate，最后由全新reviewer接收F04。
次生FileHandle teardown已由D19在`f15c210`给出DESIGN GO，F17已合流。F05等待F04
narrow receive，
F03B/F03C仍锁定至R05 PASS。

唯一权威设计是 `doc/architecture/package-service-contract-deployment.md`，重点 §1–§5、§6.2、
§9–§15。本文只冻结Phase 05的执行DAG、实现层authoring/storage/control决策、写入
ownership和验收证据，不改变四对象、两类调用或InProcessBoundary语义。

## 1. 基线与已关闭的实现决策

- Skiff基线：`5c3322ac3116ac98c4407de4396562ff632ed7b5` / tree `60321ab8c4fa11e6877a94d44b3e2d078fd428ac`。
- `skiff-packages`基线：`5defc94161cee14def1a6bbb340308004e65b741`。
- `internals`基线：`4b04e744f430f49f1ed9c76dfebfeb2a1ed5d7d2`。
- source authoring一次性收敛为：`package.yml` + `api.yml` + `.skiff` 只属于Package；
  `contract.yml` 是code-free ServiceContractDefinition；`deployment.yml` 是source-free deployment
  authoring；`assembly.yml` 只列environment与root deployments。旧 `service.yml` 不按兼容格式读取。
- `package.yml` 用顶层 `contracts` 声明contract compile coordinate/alias；编译时从已发布
  ServiceContract得到exact protocol identity并写入PackageArtifact。provider package也用contract-owned
  types，不用package-local nominal type伪装contract type。
- 本地activation使用独立的strict `EnvironmentActivationState` operational record；它不是artifact或第五个
  domain object。record包含唯一`committed { generation, assemblyRef }`与至多一个
  `pending { activationId, expectedGeneration, candidateGeneration, assemblyRef, participantReplicaIds }`。
  prepare CAS只创建pending且不移动committed；runtime全部resolve/load/link/admit到staged context并返回exact
  ACK后，router coordinator才CAS commit。prepare/reject/disconnect时按activationId abort，committed tuple不变；
  commit前再次确认非空participant set全部连接且staged；commit后旧replicas只drain in-flight，新请求等待/使用
  与committed tuple一致的registration。通知可幂等重放，controller/runtime重启按exact record向前收敛，
  不猜latest、不把新请求送回旧generation。
- router control wire只有`prepare/prepared|reject/commit/abort/register`，携带environment、activation id、
  expected/candidate generation、exact assembly ref与replica id；runtime只在committed generation激活并注册。
  新请求在同一committed assembly的healthy replicas间调度，不按service/build/target分开注册。
- ingress selector保持设计已有 `(protocol, host, method?, path)`；外部service使用唯一Host。
  `X-Skiff-Service` / `X-Skiff-Version` / query selector / rewrite-to-service在production选择路径删除；
  不改RuntimeAssembly schema来容纳legacy selector。
- publish只是四种typed artifact的immutable write + typed pointer CAS操作；不产生Publication、
  common artifact kind或archive shim。历史本地/registry数据不兼容读取，也不在本阶段破坏性删除。
- canonical local CLI拼写冻结为 `package|contract|deployment|assembly build <root>
  --artifact-root <dir> --json`；四对象remote write使用各自`publish`命令，environment切换只用
  `assembly activate <root> --artifact-root <dir> --expected-generation <n> --json`。旧`service`
  build/publish/dev命令不作alias。`assembly activate`只请求router coordinator执行上述transaction，不直接
  CAS committed pointer。T02负责实现这些入口，后续任务不得另造脚本级语义。
- 本地actual service Host固定为 `account.skiff.localhost`、`registry.skiff.localhost`、
  `codex-relay.localhost`、`aihub.localhost`、`agine.localhost`；这是现有IngressSelector字段的取值，
  不是新schema。

T01会在上述边界内冻结精确JSON字段、目录路径、CLI命令及router↔runtime wire
fixture；这些属于设计明确留给实现的选择。T01不得引入第五个domain object或改写
canonical artifact schema。

## 2. 阶段完成标准

1. 四对象都有唯一strict reader/writer/path/identity owner；任意unknown field、tamper、missing ref、
   cross-root path或partial record都在trust boundary失败。
2. `contract publish -> package compile -> deployment project -> assembly resolve -> activate` 可以分步执行；
   package compile不读provider package/deployment，deployment不读source/AST。
3. dev sync/watch将watch registry中的package/contract/deployment roots组成一个完整assembly，先写
   immutable records，再请求router执行prepare/admit/commit transaction。任一pre-commit失败只abort pending，
   不改committed active generation。
4. router协调durable activation state，runtime通过production resolver加载/验证/链接/admit staged完整assembly；
   所有participant ACK后才commit，只有观察到committed record才原子激活并注册exact assembly generation。
5. 两个runtime replica加载相同assembly identity；Host ingress到任一replica都得到同一业务
   结果。pre-commit reject/abort保留旧committed generation，in-flight request/stream保持原generation pin。
6. test-runner/package-test直接编译PackageArtifact，必要service test使用canonical contract/deployment/
   assembly fixture；不再构造PackageUnit/ServiceUnit/synthetic service assembly。
7. `skiff-packages` 使用canonical package build/test/store resolver，没有自制publication path codec；
   `internals` registry存储四种artifact/pointer，actual services只用contract types与Host ingress。
8. production legacy DTO、reader/writer、route selector、converter、fallback及stale docs归零；结构checker
   能对rename/move/duplicate/omission/test-only camouflage做mutation。

## 3. 三波DAG与执行批次

```text
Wave 1 / Batch A：shared authoring-storage-control checkpoint
  D01 phase-plan review
    └─► T01 canonical ecosystem checkpoint ─► R01 independent checkpoint acceptance
           FAIL@0cebf349 ─► F01 shared checkpoint repair ─► combined repair probe ─► R01窄复验
             FAIL@128af4a7 ─► D02 activation parity bounded audit ─► F02 repair wave
               ─► combined repair probe ─► R01第三次窄复验 PASS@c168b1dc

Wave 2 / Batch B：R01 PASS后Skiff consumers同级扇出（按worker slot滚动调度）
  T02 authoring / registry client / CLI / dev sync / watch ─┐
  T03 router active-assembly + host ingress cutover       ├─► I02 combined probe ─► R02
  T04 runtime resolver / admission / replica registration├
  T05 test-runner / package-test / fixtures
    └─► RECEIVE FAIL@f8ad689 ─► D04 bounded design ─► F04 partial repair
          ├─► D07 callable-effects audit ─► F06 compiler repair ─► R06 FAIL@2982cd8 ─► F06A field repair ─► R06 PASS@fbf634d
          │     D08 native-effects audit ───────────────────────────────────────────────────────────┴─► F07 exact native semantics ─► R07 PASS@bd13867
          │                                                                                               └─► F04 implementation checkpoint ─► shared lock ─► F08 ─► R08 PASS@c5ec7ea
          │                                                                                                     └─► F04 Host probe NO-GO@1dc1d7a ─► D10 ─► F04A checkpoint@7f36810
          │                                                                                                           └─► D11 ─► F09 ─► R10 PASS@84e33dd ─► F04A Host FAIL@ff7a4df
          │                                                                                                                                 └─► D12 ─► F10 ─► R11 PASS@47d9259 ─► F04A Host FAIL@efb2bbb
          │                                                                                                                                                                      └─► F04B@c06e115 ─► D13 ─► F11 ─► R12 PASS@a9ef444
          │                                                                                                                                                                                                   └─► D14 ─► F12 Router ─┐
          │                                                                                                                                                                                                              F13 Runtime ─┴─► R13 PASS@02b97ff ─► D15 ─► F14 ─► R14 PASS@629f1c8
          │                                                                                                                                                                                                                                                      └─► D16 ─► F15 ─► R15 FAIL@b7e0f4f ─► D17 ─► F15A@e3a0d78 ─► F04 Host FAIL@40ed693
          │                                                                                                                                                                                                                                                                                                                                                 ├─► D18 ─► F16A ─┬─► F16B ─┐
          │                                                                                                                                                                                                                                                                                                                                                 │                 └─► F16C ─┤
          │                                                                                                                                                                                                                                                                                                                                                 └─► D19 ─► F17 ───────────────────────┤
          │                                                                                                                                                                                                                                                                                                                                                                   D20A–F audit@e786671 ─► F18A–I ─► I16 PASS ─► R18A FAIL ─► F18J ─► I16/R15/R18/H18/R16 PASS ─► G16 pre-Host FAIL ─► D25 ─► F19A/B ─► replacement evidence ─► second G16 ─► new F04 receive
          └─► D05 canonical WS authoring audit ─────────────────────────────────────────────────────┐

  R02 pre-review@b47ddf7 findings
    └─► F03A shared binary/request/store seam ─► R02A FAIL@a7566bb
          └─► D03 canonical optional-field parity audit ─► F03A1 bounded repair
                └─► R02A second FAIL@5715497 ─► D06 bounded raw/normalization audit
                      └─► F03A2 convergence ─► request combined probe ─► R02A third PASS ─┐
  F04 narrow receive PASS + D05 complete ───────────────────────────────────────────────────────────┴─► F05 typed unified WS ABI ─► R05
                                                                                              ├─► F03B remaining Router store/gateway/pin ─┐
                                                                                              └─► F03C Runtime startup/admission/pin      ├─► lock refresh ─► I02 ─► R02

Wave 3 / Batch C：R02 PASS的exact Skiff checkpoint后terminal/external扇出
  T06 Skiff legacy deletion + checker + canonical docs ─────┐
  T07 skiff-packages consumer cutover                  ─────├
  T08 Internals registry/platform cutover              ─────┤
  T09A Codex contract ─┐
  T09B AIHub contract ─┤─► T09D shared Internals workflow ─► R09 ─┬─► T10 Codex Relay
  T09C Agine contract ─┘                                      ├─► T11 AIHub
                                                             └─► T12 Agine + clients

  T07 + T08 + T10–T12 ─► T09E final Internals assembly/workflow
  T06 + T09E ─► I03 cross-repo combined probe ─► R03 ─► T13 pre-merge final gate ─► A01
      └─► each repo one main merge ─► V01 post-merge stable verification ─► cleanup
```

Wave 2的四个节点没有语义依赖；平台只有三个worker slot，因此前三个先启动，任一
完成后立即用新Agent启动第四个，不把调度限制伪装成DAG依赖。Wave 3需要一个短的
Internals contract/workflow checkpoint，因为Agine编译必须只依赖已冻结AIHub ServiceContract；这是真实
contract-first依赖，不用复制临时descriptor伪并行。三个contract owner可并行，合流后由T09D
唯一修改Internals共享package-store/isolated graph framework；T10–T12落地实际deployments后，T09E才拥有
production `assembly.yml`及完整closure，避免上游checkpoint虚构尚不存在的deployment identity。
T06/T07/T08在该链路期间继续执行，因此不增加第四个架构实现波次。

Batch A退出点是R01 PASS的Skiff implementation checkpoint；Batch B退出点是R02 PASS且可供
外部repo构建的exact Skiff commit；Batch C退出点是三个repo都clean、无在途写入且风险
probe通过的pre-acceptance candidate。批次之间只依赖task contract、exact commits及证据ledger。
R03/T13/A01均在main merge前完成。Internals规则禁止linked worktree运行AIHub/Agine build/dev/start，
因此main合并后的stable provider/list/chat只由V01执行；它是部署后验证，不替代或放宽pre-merge gate。

R02 PASS后，主integration owner在Skiff integration HEAD不变且clean时执行：

```bash
P5_R02_COMMIT="$(git rev-parse HEAD)"
git worktree add --detach /Users/geek/workspace/skiff-p5-r02-checkpoint "$P5_R02_COMMIT"
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
```

并记录exact commit/tree。T07–T12/T09A–E只读该不可移动detached checkpoint，所有Cargo/generated output
放在task临时目录；每次证据前后都核对HEAD、tree与clean。T06继续修改Skiff integration不会改变外部
consumer输入。最终I03/T13才改用包含T06的frozen Skiff integration tree。checkpoint worktree保留到V01 PASS。

## 4. 任务索引

| ID | 任务 | 依赖 | 风险 / 验收组 |
| --- | --- | --- | --- |
| D01 | [Phase-plan review](tasks/P5-D01-phase-plan-review.md) | docs checkpoint | 独立只读 |
| T01 | [Canonical ecosystem checkpoint](tasks/P5-T01-canonical-ecosystem-checkpoint.md) | D01 PASS | 高；shared schema/storage/control |
| R01 | [Checkpoint acceptance](tasks/P5-R01-ecosystem-checkpoint-acceptance.md) | T01 exact commit | 高；独立只读 |
| F01 | [R01 shared checkpoint repair](tasks/P5-F01-r01-shared-checkpoint-repair.md) | R01 FAIL at `0cebf349` | 高；path/control/alias shared owner repair |
| D02 | [Activation parity bounded audit](tasks/P5-D02-activation-parity-bounded-audit.md) | R01 second FAIL at `128af4a7` | 独立只读；第三次verdict熔断前置 |
| F02 | [Activation parity convergence](tasks/P5-F02-activation-parity-convergence.md) | D02 complete | 高；raw/typed codec一次修复波次 |
| T02 | [Authoring/tooling cutover](tasks/P5-T02-authoring-tooling-cutover.md) | R01 PASS | 高；tooling consumer |
| T03 | [Router cutover](tasks/P5-T03-router-active-assembly-cutover.md) | R01 PASS | 高；control/ingress |
| T04 | [Runtime provisioning](tasks/P5-T04-runtime-assembly-provisioning.md) | R01 PASS | 高；reload/admission/replica |
| T05 | [Test infrastructure cutover](tasks/P5-T05-test-infrastructure-cutover.md) | R01 PASS | 中高；test production owner |
| D04 | [Test infrastructure repair design](tasks/P5-D04-test-infrastructure-repair-design.md) | T05 receive FAIL at `f8ad689` | 独立只读；冻结真实执行/CLI seam |
| F04 | [Test infrastructure integration repair](tasks/P5-F04-test-infrastructure-integration-repair.md) | D04 complete | 高；T02/T05 test seam窄修复 |
| D05 | [Canonical WebSocket authoring audit](tasks/P5-D05-canonical-websocket-authoring-audit.md) | F04 risk probe | 独立只读；冻结production WS ABI |
| D07 | [Callable effects fixture boundary audit](tasks/P5-D07-callable-effects-fixture-boundary-audit.md) | F04 real-consumer blocker | 独立只读；冻结compiler transfer缺口 |
| F06 | [Callable effects exact dependency transfer](tasks/P5-F06-callable-effects-exact-dependency-transfer.md) | D07 complete | 高；compiler/source窄修复 |
| R06 | [Callable effects transfer acceptance](tasks/P5-R06-callable-effects-transfer-acceptance.md) | F06 exact commit | 高；独立只读 |
| F06A | [Exact callee field repair](tasks/P5-F06A-exact-callee-field-repair.md) | R06 FAIL at `2982cd8` | 高；exact callee wrapper窄修复 |
| D08 | [Exact native callable effects audit](tasks/P5-D08-exact-native-callable-effects-audit.md) | F04 std-suite blocker | 独立只读；冻结native descriptor边界 |
| F07 | [Exact native callable effects](tasks/P5-F07-exact-native-callable-effects.md) | R06 PASS + D08 | 高；shared native semantics窄修复 |
| R07 | [Exact native callable effects acceptance](tasks/P5-R07-exact-native-callable-effects-acceptance.md) | F07 exact commit | 高；独立只读 |
| D09 | [Host test-runtime cycle audit](tasks/P5-D09-host-test-runtime-cycle-audit.md) | F04 Host blocker after R07 | 独立只读；冻结DAG解环 |
| F08 | [Host test-runtime seam repair](tasks/P5-F08-host-test-runtime-seam-repair.md) | F04 implementation checkpoint + shared lock | 高；Host legacy seam删除 |
| R08 | [Host test-runtime seam acceptance](tasks/P5-R08-host-test-runtime-seam-acceptance.md) | F08 exact commit | 高；独立只读 |
| D10 | [F04 real Host completion audit](tasks/P5-D10-f04-real-host-completion-audit.md) | F04 Host NO-GO at `1dc1d7a` | 独立只读；冻结environment/fixture缺口 |
| F04A | [Real Host execution completion](tasks/P5-F04A-real-host-execution-completion.md) | D10 complete | 高；environment与runnable fixture窄修复 |
| D11 | [Router control-wire bootstrap audit](tasks/P5-D11-router-control-wire-bootstrap-audit.md) | F04A Host NO-GO at `031c6b8` | 独立只读；冻结endpoint/DAG缺口 |
| F09 | [Router control-wire bootstrap repair](tasks/P5-F09-router-control-wire-bootstrap-repair.md) | D11 complete | 高；统一endpoint/session前置修复 |
| R10 | [Router control-wire bootstrap acceptance](tasks/P5-R10-router-control-wire-bootstrap-acceptance.md) | F09 exact commit | 高；独立只读 |
| D12 | [Runtime committed bootstrap audit](tasks/P5-D12-runtime-committed-bootstrap-audit.md) | F04A Host FAIL at `ff7a4df` | 独立只读；冻结cold-start/reconnect缺口 |
| F10 | [Runtime committed bootstrap repair](tasks/P5-F10-runtime-committed-bootstrap-repair.md) | D12 complete | 高；F03C committed recovery前置修复 |
| R11 | [Runtime committed bootstrap acceptance](tasks/P5-R11-runtime-committed-bootstrap-acceptance.md) | F10 exact commit | 高；独立只读 |
| F04B | [Source suite runner disambiguation](tasks/P5-F04B-source-suite-runner-disambiguation.md) | R11 PASS + F04A Cargo 101 | 低；canonical runner argv窄修复 |
| D13 | [Std exact callable effects audit](tasks/P5-D13-std-exact-callable-effects-audit.md) | F04A std boundary blocker at `c06e115` | 独立只读；冻结native/receiver覆盖 |
| F11 | [Std exact callable effects repair](tasks/P5-F11-std-exact-callable-effects-repair.md) | D13 complete | 高；production semantics/target facts收敛 |
| R12 | [Std exact callable effects acceptance](tasks/P5-R12-std-exact-callable-effects-acceptance.md) | F11 exact commit | 高；独立只读 |
| D14 | [Canonical unary request consumer audit](tasks/P5-D14-canonical-unary-request-consumer-audit.md) | F04A flat-header blocker at `2d74b2c` | 独立只读；冻结双端consumer |
| F12 | [Router canonical unary consumer](tasks/P5-F12-router-canonical-unary-consumer.md) | D14 complete | 高；Router writer/dispatch consumer |
| F13 | [Runtime canonical unary bridge](tasks/P5-F13-runtime-canonical-unary-bridge.md) | D14 complete | 高；Runtime decoder/trust bridge |
| R13 | [Canonical unary request acceptance](tasks/P5-R13-canonical-unary-request-acceptance.md) | F12 + F13 exact integration | 高；独立combined只读 |
| D15 | [Linked FileRef semantics audit](tasks/P5-D15-linked-file-ref-semantics-audit.md) | F04A Host link reject at `02b97ff` | 独立只读；冻结locator/semantic差异 |
| F14 | [Linked FileRef semantics repair](tasks/P5-F14-linked-file-ref-semantics-repair.md) | D15 complete | 中；linked-program窄修复 |
| R14 | [Linked FileRef semantics acceptance](tasks/P5-R14-linked-file-ref-semantics-acceptance.md) | F14 exact commit | 高；独立只读 |
| D16 | [Post-commit readiness audit](tasks/P5-D16-post-commit-readiness-audit.md) | F04A first-request 503 at `bcbdc2c` | 独立只读；冻结registration空窗 |
| F15 | [Test runtime readiness barrier](tasks/P5-F15-test-runtime-readiness-barrier.md) | D16 complete | 中；test runtime唯一request barrier |
| R15 | [Test runtime readiness acceptance](tasks/P5-R15-test-runtime-readiness-acceptance.md) | F15 exact commit | 高；独立只读 |
| D17 | [Readiness hardening audit](tasks/P5-D17-readiness-hardening-audit.md) | R15 FAIL at `b7e0f4f` | 独立只读；冻结deadline/schema/模块边界 |
| F15A | [Readiness hardening repair](tasks/P5-F15A-readiness-hardening-repair.md) | D17 complete | 中；same-base单一重建 |
| D18 | [Canonical platform source trust audit](tasks/P5-D18-canonical-platform-source-trust-audit.md) | F04 Host reserved-id at `40ed693` | 独立双owner只读；冻结shared trust root |
| F16A | [Compiler platform source context](tasks/P5-F16A-compiler-platform-source-context.md) | D18 complete | 高；shared compiler trust checkpoint |
| F16B | [Compiler platform source transport](tasks/P5-F16B-compiler-platform-source-transport.md) | F16A checkpoint | 高；authoring consumer |
| F16C | [Test runner platform source transport](tasks/P5-F16C-test-runner-platform-source-transport.md) | F16A checkpoint | 高；test production consumer |
| D20 | [F04 production path closure audit](tasks/P5-D20-f04-production-path-closure-audit.md) | F04跨层收敛熔断 | 六个新只读审计/预读节点；root唯一汇总 |
| D20R | [F04 production path closure result](tasks/P5-D20-f04-production-path-closure-audit-result.md) | D20A–F complete | 14跳闭合矩阵；批量repair DAG |
| R15A | [Readiness reacceptance result](tasks/P5-R15-readiness-reacceptance-result.md) | D20 evidence audit | FAIL；语义通过、F18I Clippy blocker |
| F18A | [Prelude source containment](tasks/P5-F18A-prelude-source-containment.md) | D20 closed | 高；compiler trust owner |
| F18B | [Platform context pre-read guard](tasks/P5-F18B-platform-context-pre-read-guard.md) | D20 closed | 高；authoring/runner ordering |
| F18C | [Isolated runtime provenance/readiness](tasks/P5-F18C-isolated-runtime-provenance-readiness.md) | D20 closed | 高；test runtime boundary |
| F18D | [Router file activation CAS](tasks/P5-F18D-router-file-activation-cas.md) | D20 closed | 高；durable concurrency |
| F18E | [Supervisor process lifecycle closure](tasks/P5-F18E-supervisor-process-lifecycle-closure.md) | D20 closed | 高；process/PID/FD owner |
| F18F | [Gate resource ownership](tasks/P5-F18F-gate-resource-ownership.md) | D20 closed | 高；no-clobber cleanup |
| F18G | [Host negative result harness](tasks/P5-F18G-host-negative-result-harness.md) | D20 closed | 中；focused evidence设施 |
| F18H | [Compiler test-only platform context](tasks/P5-F18H-compiler-test-platform-context-transport.md) | D22 closed | 中；verify compile blocker |
| F18I | [Host fixture Clippy closure](tasks/P5-F18I-host-fixture-clippy-closure.md) | R15 reaccept FAIL | 低；candidate hygiene |
| F18J | [Authoring pre-store platform guard](tasks/P5-F18J-authoring-pre-store-platform-guard.md) | R18A FAIL | 低；同owner顺序与副作用回归 |
| I16 | [Platform source shared-target combined probe](tasks/P5-I16-platform-source-shared-target-probe.md) | D20/F18J repair wave merged | 每个exact candidate唯一cheap combined owner；不跑Host |
| R15B | [Readiness Clippy reacceptance](tasks/P5-R15B-readiness-clippy-reacceptance.md) | I16 PASS | 低；只复验R15A exact blocker |
| R18A | [Compiler trust acceptance](tasks/P5-R18A-compiler-trust-acceptance.md) | I16 PASS | 高；独立只读 |
| R18B | [Router file CAS acceptance](tasks/P5-R18B-router-file-cas-acceptance.md) | I16 PASS | 高；独立只读 |
| R18C | [Resource lifecycle acceptance](tasks/P5-R18C-resource-lifecycle-acceptance.md) | I16 PASS | 高；独立只读 |
| H18 | [Host focused-negative execution](tasks/P5-H18-host-focused-negative-execution.md) | I16 PASS | 唯一focused-negative owner；不计full |
| R16 | [F04 production path narrow acceptance](tasks/P5-R16-platform-source-trust-acceptance.md) | I16 PASS + D20 closed | 高；全新独立只读 |
| G16 | [F04 real Host gate](tasks/P5-G16-f04-real-host-gate.md) | R16 PASS | 当前周期唯一完整Host owner |
| G16R | [F04 real Host gate result](tasks/P5-G16-f04-real-host-gate-result.md) | G16 first full-mode call | FAIL；Host前artifact comparator |
| D25 | [G16 pre-Host closure audit result](tasks/P5-D25-g16-pre-host-closure-audit-result.md) | G16 FAIL | 三个全新只读审计；闭合被遮挡范围 |
| F19A | [Gate artifact/evidence convergence](tasks/P5-F19A-gate-artifact-evidence-convergence.md) | D25 complete | 高；test-only gate evidence owner |
| F19B | [Isolated workspace ownership](tasks/P5-F19B-isolated-workspace-ownership.md) | D25 complete | 高；inner no-clobber owner |
| R19A | [Gate evidence acceptance](tasks/P5-R19A-gate-evidence-acceptance.md) | v4 I16 PASS | 高；独立只读 |
| R19B | [Isolated workspace acceptance](tasks/P5-R19B-isolated-workspace-acceptance.md) | v4 I16 PASS | 高；独立只读 |
| D19 | [Supervisor log-handle teardown audit](tasks/P5-D19-supervisor-log-handle-teardown-audit.md) | F04 cleanup secondary at `40ed693` | 独立只读；不阻塞F16启动 |
| F17 | [Supervisor log-handle lifecycle repair](tasks/P5-F17-supervisor-log-handle-lifecycle.md) | D19 DESIGN GO | 中；独立resource lifecycle owner |
| F03A | [Router/runtime shared seam](tasks/P5-F03A-router-runtime-shared-seam.md) | R02 pre-review findings | 高；binary/header/store checkpoint |
| R02A | [Router/runtime seam acceptance](tasks/P5-R02A-router-runtime-seam-acceptance.md) | F03A exact commit | 独立只读；不作R02 verdict |
| D03 | [Canonical request optional parity audit](tasks/P5-D03-canonical-request-optional-parity-audit.md) | R02A FAIL at `a7566bb` | 独立只读；冻结完整字段矩阵 |
| F03A1 | [Canonical request optional parity repair](tasks/P5-F03A1-canonical-request-optional-parity-repair.md) | D03 complete | 高；共享request codec/corpus窄修复 |
| D06 | [Canonical request raw/normalization audit](tasks/P5-D06-canonical-request-raw-normalization-audit.md) | R02A second FAIL at `5715497` | 独立只读；第三次verdict熔断审计 |
| F03A2 | [Canonical request raw/normalization convergence](tasks/P5-F03A2-canonical-request-raw-normalization-convergence.md) | D06 complete | 高；shared raw/typed/default一次收敛 |
| F05 | [Canonical WebSocket ingress authoring](tasks/P5-F05-canonical-websocket-ingress-authoring.md) | F04 receive + R02A PASS | 高；typed WS shared checkpoint |
| R05 | [Canonical WebSocket ingress acceptance](tasks/P5-R05-canonical-websocket-ingress-acceptance.md) | F05 exact commit | 高；独立只读 |
| F03B | [Router integration repair](tasks/P5-F03B-router-integration-repair.md) | R05 PASS | 高；unified endpoint/store/pin |
| F03C | [Runtime integration repair](tasks/P5-F03C-runtime-integration-repair.md) | R05 PASS | 高；startup/admission/pin |
| I02 | [Skiff combined probe](tasks/P5-I02-skiff-combined-probe.md) | T02–T05 merged | 主integration owner；便宜动态probe |
| R02 | [Skiff cutover acceptance](tasks/P5-R02-skiff-cutover-acceptance.md) | I02 PASS | 高；批次验收 |
| T06 | [Skiff terminal deletion/checker/docs](tasks/P5-T06-skiff-terminal-cleanup.md) | R02 PASS | 中高；terminal owner |
| T07 | [skiff-packages cutover](tasks/P5-T07-skiff-packages-cutover.md) | R02 exact Skiff | 中；外部consumer |
| T08 | [Internals registry/platform](tasks/P5-T08-internals-registry-platform.md) | R02 exact Skiff | 高；registry/release |
| T09A | [Codex contract](tasks/P5-T09A-internals-codex-contract.md) | R02 exact Skiff | 高；contract ABI checkpoint |
| T09B | [AIHub contract](tasks/P5-T09B-internals-aihub-contract.md) | R02 exact Skiff | 高；contract ABI/schema |
| T09C | [Agine contract](tasks/P5-T09C-internals-agine-contract.md) | R02 exact Skiff | 高；contract/API owner split |
| T09D | [Internals canonical workflow](tasks/P5-T09D-internals-canonical-workflow.md) | T09A–T09C merged | 高；shared workflow checkpoint |
| R09 | [Internals contract/workflow acceptance](tasks/P5-R09-internals-contract-acceptance.md) | T09D exact commit | 高；独立只读 |
| T10 | [Codex Relay](tasks/P5-T10-internals-codex-relay.md) | R09 PASS | 高；service/deployment/host |
| T11 | [AIHub](tasks/P5-T11-internals-aihub.md) | R09 PASS | 高；service boundary/schema |
| T12 | [Agine + clients](tasks/P5-T12-internals-agine-clients.md) | R09 PASS + T07 exact packages | 高；service/chat ingress |
| T09E | [Final Internals assembly/workflow](tasks/P5-T09E-internals-final-assembly.md) | T07/T08 + T10–T12 merged | 高；完整环境closure |
| I03 | [Cross-repo combined probe](tasks/P5-I03-cross-repo-combined-probe.md) | T06 + T09E exact trees | 主integration owner；actual assembly |
| R03 | [Cross-repo ecosystem acceptance](tasks/P5-R03-ecosystem-acceptance.md) | I03 PASS | 高；单一verdict |
| T13 | [Unique final gate](tasks/P5-T13-phase-integration-gate.md) | R03 PASS + frozen candidate | 唯一昂贵gate owner |
| A01 | [Independent stage acceptance](tasks/P5-A01-stage-acceptance.md) | T13 ledger | 独立最终验收 |
| V01 | [Post-merge stable verification](tasks/P5-V01-post-merge-stable-verification.md) | A01 PASS + one merge/repo | 唯一stable/live owner |

## 5. 写入 ownership

- T01只在新的canonical authoring/storage/control模块、四类typed record fixture、production
  `RuntimeAssemblyContentResolver`与router↔runtime cross-language fixture内建立检查点。它可对
  `artifact-model` / `artifact-identity` / `compiler input` / `deployment` / `runtime loader` / `router protocol`
  做最小public seam，但不迁移任何现有consumer、不删legacy模块。`Cargo.toml`/`Cargo.lock`
  在Wave 1归T01独占。
- F01只关闭R01@`0cebf349`的三个shared checkpoint blocker：单射coordinate codec、tooling→router
  activation request与Rust/TS state/control parity、dependency alias单一leaf owner。它不得实现任何
  T02–T05 consumer。合流后主integration owner只运行
  `cargo test -p skiff-compiler-input contract_dependencies`、
  `cargo test -p skiff-runtime-loader runtime_assembly`与
  `pnpm --filter @skiff/router type-check`作为一次combined repair probe；通过后才触发原R01 reviewer窄复验。
- D02不产生verdict；它在R01第二次FAIL后穷举activation request/state/control的token、generation、raw JSON、
  required-nullable、participant与variant evidence。F02只写这些shared codec owner，不再修改path/alias或实现
  consumer。F02合流后的唯一combined probe是：
  `cargo check -p skiff-artifact-identity -p skiff-deployment -p skiff-runtime-loader`、
  `pnpm --filter @skiff/router type-check`、
  `node cross-system-fixtures/package-service-ecosystem/verify.mjs --combined-probe`和`git diff --check`；
  全部PASS后才允许R01第三次窄复验。
- T02独占 `compiler/**` 的binary/authoring consumer、`scripts/skiff*.mjs`、`scripts/lib/**`中的
  build/publish/store/dev sync/watch、对应tests。不改router/runtime/test-runner，不在85k/62k旧
  大文件继续塞入新owner；新职责拆到模块。
- T03独占 `router/**`：active assembly snapshot/reload、host ingress、assembly replica dispatch、
  health/control protocol TS consumer。不改Rust runtime或tooling。
- T04独占 `runtime/transport/**`、`runtime/loader/**`、`runtime/host/**`的production resolver/
  admission/generation/registration/lifecycle；不改`runtime/package-test`/test-runner/router/scripts。
- T05独占 `test-runner/**`、`runtime/package-test/**`、test-only canonical fixture builders及isolated test
  runtime harness；不改runtime host production、router或tooling production。T05接收FAIL后，D04只读冻结真实
  canonical dependency/base assembly数据流与CLI disposition；F04可串行修改T05 owned paths及T02
  `scripts/skiff.mjs`/isolated/verify直接caller，删除失效参数并接通同一canonical runner。F04不改compiler stream、
  runtime host或shared wire。D09确认该consumer与F04 receive形成依赖环；F04 implementation checkpoint可先合入但
  不作receive verdict，由root唯一刷新shared lock后，F08删除runtime-host legacy package-test consumer及两个旧codec
  caller，R08 PASS后F04才运行真实Host probe。F03C不得恢复该seam。
- D07只读穷举F04真实consumer被拒绝的callable-effects来源；F06串行独占compiler/source的exact dependency
  call-position、全detached contract descriptor transfer与直接标量参数字段写facts。不改projection/lowering/
  artifact/runtime。R06 PASS后F04才把同一fail-closed fixture翻为isolated最终正例，不在F04复制effects规则。
- D08只读确认native signature有shared identity但无effects owner；F07在R06后串行建立缺省Unknown的稀疏exact
  native semantics，只批准四个context-free string scalar，并让compiler/runtime交叉校验同一binding key。R07 PASS
  后F04才恢复canonical std source suite；未知/crypto/capability native不因本修复放宽。
- D09只读确认F04 Host证据与F03C依赖成环，并冻结F08删除legacy Host package-test seam。F04 implementation
  checkpoint合流后，root integration owner独占一次Cargo.lock刷新并冻结exact shared-lock commit；F08只改Host
  consumer、不得再改manifest/lock，R08 PASS后F04原fixture才可完成receive。该前置修复不实现F03C的startup、
  lifecycle、request trust boundary、drain或typed WS职责。
- D10只读确认R08后isolated Router environment仍在script config链丢失，且helper/service场景没有进入real runner。
  F04A只在local config/renderer caller与test infrastructure fixture/preparer内关闭两项；source registry、production
  writer、Host/wire与WebSocket smoke保持不变。真实Host结果是F04 receive硬门禁。
- F04A checkpoint修复上述两项后，D11只读确认production Router仍实例化缩减endpoint，拒绝Runtime首帧binary
  capabilities并以text处理binary-only activation，且health混淆capability connection与committed registration。
  F09/R10只从F03B提前拆出统一endpoint/session/bootstrap职责；不改Runtime/shared wire/store/gateway，不提前实现
  F03C。R10 PASS仅恢复F04A真实probe；F03B其余职责仍等待R05。
- R10后的真实probe确认Router exact committed generation-0与capability session均存在，但Runtime空admission state没有
  durable committed reader，只发capabilities而不注册replica。D12/F10/R11只从F03C提前拆出strict Runtime config、
  每次连接前的exact committed recovery与共享admission/publication primitive；不把capability升级为participant、不伪造
  online transaction，也不提前实现request trust boundary/WS/drain。R11 PASS仅恢复F04A真实probe。
- R11后的真实probe已越过isolated ready并进入std；F04B只在source-suite direct caller显式选择
  `skiff-test-runner` binary并补直接test，不改任何production/runtime/fixture/manifest。F04B后仍以完整Host结果为门禁。
- D13只读确认完整std的crypto/time/date/duration/number native与Date/Duration receiver缺exact production semantics/
  target facts；boundary按设计正确fail closed。F11/R12只扩稀疏exact registry并收敛source facts→lowering→runtime parity，
  不改std/runner/boundary/fixture，不放宽unknown/dynamic/mutable/capability native。R12后恢复F04A真实probe。
- R12后的真实probe确认F03A2 shared request codec正确但production Router/Runtime consumer仍分别发送flat header与使用
  legacy decoder。F12/F13从F03B/F03C并行提前拆出normal HTTP unary writer/dispatch与strict active-route bridge，R13
  combined验收；不改shared codec，不实现WS/serverStream/httpAdapter/test doubles/drain。R13后恢复F04A真实probe。
- R13后的真实suite已执行std 11/11，Host assembly在linked-program把storage locator误当semantic FileIR target字段时
  fail closed。D15/F14/R14只让callable/executable target matcher忽略artifactPath，identity/module/present hash/index
  继续严格；不回改authoring/identity/loader/test normalizer/fixture。R14后由原owner原样运行唯一F04 Host gate。
- R14后的原样gate已完成gen2 commit/register，但activation 2xx先于healthy registration可见，test runner首请求落入
  dispatch空窗。D16/F15/R15只在test runtime的唯一业务request前等待exact active tuple、healthy replica与matching
  capability；不改Router receipt、固定sleep或重试业务request。R15后再由原owner原样运行F04 Host gate。
- R15首次拒绝F15的无界DNS、宽松pending/UTF-8与1273行混合职责。D17/F15A从原base重建，复用activation连接
  peer实现barrier内零DNS和absolute I/O deadline，以production activation validate收敛pending invariants，并拆分HTTP/
  readiness/wire模块。F15A checkpoint只由root跑一次combined test，R15窄复验后才恢复F04 gate。
- F15A后的原样gate在编译std前暴露编译期platform path被共享Cargo target跨worktree复用。D18确认root/cwd/argv/
  registry均正确，禁止以clean cache、隔离target或reserved-id放宽修复。F16A唯一建立显式canonical
  `CompilerPlatformSources`并移除input/source/prelude的ambient path；F16B与F16C只迁移各自transport consumer，不得
  各造resolver。二者与F17合流后，D20A–F按source/artifact、activation/readiness、request/eval/cleanup及证据缺口闭合
  14跳真实路径；root一次汇总为F18A–I九个互斥repair owner。首次combined后R18A只发现authoring pre-store顺序缺口，
  F18J作为同owner窄后继关闭；新candidate重新执行一次compiler merge probe与A-built/Fresh-B shared-target cheap combined；
  全新R15B与三组窄验收及H18均PASS后启动新R16，G16再运行一次完整Host并把
  同一ledger交新的F04 reviewer。首次G16在Host前因gate comparator假阴性FAIL，D25A–C已闭合Cargo artifact、
  full evidence state machine与inner workspace ownership；F19A/B并行合流后必须重建v4 combined及受影响窄证据，
  才能执行本周期第二次full-mode gate。第三次前必须重新审计剩余范围并说明原因。
- 同一失败cleanup中的FileHandle异常是primary reserved-id之后的次生事件。D19只读冻结真实handle owner；F17在
  `skiff-instance`提取单一幂等可等待lifecycle并用真实FileHandle/短命child交错验证。不重跑Host、不阻塞F16A/B/C，
  但F17是I16硬前置且写集不得与F16C重叠。
- F03A在R02预审后串行独占Router/Runtime shared wire、compiler internal canonical-store adapter与cross-language
  fixture。R02A首次FAIL后，D03只读穷举canonical request所有optional/nested字段的两端接受集合；F03A1只改
  request shared codec、直接tests与同一cross-language corpus，不回改已PASS的activation/store，也不实现consumer。
  第二次FAIL触发D06熔断审计；F03A2只收敛raw Unicode、opaque number、decoded defaults与parser单一职责。
  request combined probe通过且R02A第三次窄复验PASS后才允许F05扩展该shared seam；R05 PASS后F03B只写Router
  consumer，F03C只写Runtime consumer。两者不得回改shared seam；所有repair与T05合流后才刷新Cargo.lock并
  运行I02，避免单侧mock再次冒充interop证据。
- D05只读冻结normal source authoring到production WS generation pin的缺口。F05在F04/R02A完成后串行独占
  typed unified WebSocket ABI，从std/compiler/deployment经shared request wire、runtime adapter/eval到Router
  assembly gateway形成一个checkpoint；不改四对象schema、activation/store或F03B/F03C的endpoint/startup owner。
  R05 PASS后F03B/F03C才可消费该ABI，后续consumer不得再回改`websocket.ingressEvent`或connect/receive规则。
- T06在R02后独占 `artifact-model`/`artifact-identity`/runtime linked/eval的旧module删除，canonical
  ecosystem checker/verify接线、`cross-system-fixtures/**`及旧reference/architecture/runtime/router文档。
  不修改已通过R02的consumer语义。F04 extra-review移交的encrypted-storage live harness、legacy service fixtures、
  runtime-live service fixtures/base capability assembly、test-runner isolation/AGENTS命令也归T06串行迁移到四对象与
  canonical CLI；它们不是F04 deleted-flag或live可执行性的兼容例外。F16C先行只拥有这些入口的
  `--platform-source-root` transport与专用tests，T06后续完整迁移必须保留，不得恢复ambient path。
- T07独占 `skiff-packages` repo。T08独占 `internals/skiff-platform/account/**`、
  `internals/skiff-platform/package-registry/**`及必要platform registry client。T09A–T09C分别只写
  各自product的code-free contract authoring/schema fixture，不改service implementation。T09D独占
  `internals/scripts/**`、`aihub/service/scripts/**`、共享package-store/build graph及AIHub/Agine的package script接线，
  但不建立production assembly authoring。
  T10独占 `internals/codex-relay/**`；T11独占 `internals/aihub/**`（排除T09D已冻结文件）；T12
  独占 `internals/agine/**`及客户端Host/WS/chat smoke（排除T09D已冻结文件）。T09E在这些任务合流后
  独占Internals root `assembly.yml`、完整closure配置、`prepare-canonical-assembly.mjs`最终接线及其tests。
- T13只做preflight、stable candidate冻结、唯一gate调度及ledger/结果文档草案；不做
  实现修复，也不操作stable。V01只在三仓各自main已完成唯一merge后操作stable并记录live ledger；
  任何candidate-code blocker停止阶段并升级，不用第二次main merge偷偷修复。

`artifact-model/src/lib.rs`、`artifact-identity/src/{lib.rs,error.rs,constants.rs}`、root `Cargo.toml`/
`Cargo.lock`、`scripts/verify*.mjs`、cross-system parity fixture都是串行集成面；任何并行Agent不得
顺手修改。

Wave 2若T02的owned `compiler/Cargo.toml`新增canonical deployment依赖，task branch仍不得提交由Cargo
机械改写的root `Cargo.lock`。T02记录exact lock diff并在提交前恢复；T02–T05全部合流后，由主integration
owner在单一HEAD刷新一次lock，确认diff只包含owned manifest对应的dependency metadata，单独提交并重建
compiler compile evidence。该串行metadata收口是I02前置，不制造T02到其它consumer的语义依赖。

I02/I03由主integration owner在clean合流commit上各执行一次且不修改文件：I02只跑一replica的Skiff
authoring→activation transaction→Host最终结果/abort rollback；I03只跑一replica的五actual-deployment
isolated assembly。T13的two-replica generic lifecycle与完整selectors不重跑这两条命令。

## 6. 最早风险探针

### Authoring / storage / release

- 先publish contract，删除/隐藏provider package后consumer package仍可compile；provider/deployment
  字段混入contract读取必须失败。
- 四种artifact往返bit-identical；identity tamper、path traversal、unknown field、missing blob/ref、
  duplicate operation/provider全部fail closed。
- activation prepare stale CAS、temp/partial record、generation rollback、assembly identity/path mismatch不创建
  pending；reject/disconnect/abort保持committed tuple，commit后replay只向前收敛。

### Router / runtime / replica

- activation candidate在resolve/load/link/admit任一步失败，router/runtime abort pending并保留旧committed
  generation及注册；全部participant staged ACK后才commit并原子切换。prepare/ACK/commit crash点可恢复。
- 两个独立runtime-home的replica注册同一AssemblyIdentity，共享PackageBuildId代码但
  不共享activation mutable owner；断开一个replica后新请求继续到另一个。
- `codex-relay.localhost` / `aihub.localhost` / `agine.localhost` 等Host可区分相同path；
  legacy service/version header/query、rewrite-to-service及缺Host均不能选中deployment。
- admission后request期间artifact resolver/filesystem spy为零；request/stream保持active generation pin。

### Tests / external consumers

- test-runner/package-test的artifact output只命中PackageArtifact/必要canonical contract/deployment/assembly，
  `ServiceUnit`/`PackageUnit`/`serviceAssembly`为零。
- `skiff-packages/track` 的package direct call使用 `alias/publicPath`，contract type仍使用 `alias.Type`；
  harness不再手工编码publication store path或指向`language/scripts`。
- AIHub等service boundary只用contract-owned schema；package-local `llm-api` nominal types通过显式wrapper
  转换，不进入ServiceContract。
- 真实registry publish/resolve/pointer history、provider list与chat smoke到达最终业务结果，不只到
  route/admission checkpoint。

## 7. 证据覆盖与gate经济性

开发Agent只运行targeted format/static/direct tests。R01验收shared schema/storage/control边界；R02验收
Skiff consumer合流；R09验收Internals code-free contract及schema closure；T09E关闭完整environment
assembly的最后共享owner；I02/I03是两个明确的批次combined-probe owner；R03是三仓ecosystem的单一
pre-gate verdict。每个修复批次合流后，先由对应integration owner运行便宜combined probe，再重验
受影响边界。

T13在冻结前完成命令展开、依赖/工具、worktree source provenance、隔离Cargo target、
stable instance identity/health、Mongo/ports、package-store symlink/watch registry及已知baseline预检。准备动作
只在冻结前执行，不冒充PASS证据。

最终候选的唯一gate计划至少包含：

```bash
# Skiff：完整non-live验证，只执行一次
pnpm --dir /Users/geek/workspace/skiff-phase-05-integration verify

# 聚焦结构与生态checker（若已被verify展开则不重复）
node /Users/geek/workspace/skiff-phase-05-integration/scripts/check-package-service-ecosystem-boundaries.mjs

# 外部repo non-live
npm --prefix /Users/geek/workspace/skiff-packages-phase-05-integration test
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
  node /Users/geek/workspace/internals-phase-05-integration/scripts/verify-phase05-ecosystem.mjs --non-live

# 冻结候选的隔离动态验收
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-package-service-ecosystem-smoke.mjs --replicas 2
```

T13必须从 `verify --list`及新checker registry确认去重后再记ledger。A01 PASS后主Agent才将
三仓分支各自一次合入main。随后V01按Internals AGENTS重建main local package store，等待同一轮
所有artifacts reload，再执行registry/provider/list/chat stable smoke，不在版本混杂时判断。

## 8. 候选成熟度与证据失效

- T01/R01只是shared implementation checkpoint，不是pre-acceptance candidate。
- F16A/B/C会使`40ed693`的F04 source-suite/Host证据失效，但不使R15 readiness代码验收失效；只有D20闭合且repair
  wave合流、I16 exact shared-target combined、R16 PASS及G16 exact Host ledger可恢复F04 receive。F17写集固定不重叠
  F16C，不额外失效其ledger。当前收敛周期同一完整probe原则上最多两次；第三次前必须重做剩余范围审计并说明原因。
- R02 PASS表示Skiff production consumers已合流，可供外部repo编译；T06旧模型删除与外部
  迁移仍在途，不得冻结最终候选。
- T06–T12与T09E合流、combined probe PASS、无在途写入/设计问题，且阶段标准已映射到
  真实入口/正负例/owner/exact commits后，才是pre-acceptance candidate。
- R03 PASS与gate preflight完成后，冻结三个repo exact commits/trees成为stability epoch。
  T13/A01不得修改候选；A01 PASS后的bit-identical merge不结束epoch。
- V01锚定各仓main merge commit/tree与对应source tree；只有stable active assembly的对象identity
  与冻结candidate一致时，pre-merge证据才可和live证据合并。V01 PASS前阶段不标记COMPLETE、
  不删除integration worktree/branch。
- 影响artifact schema/path/pointer、authoring parser、router/runtime protocol/admission、service contracts/
  deployments、checker/fixture、Cargo/lock、package store或stable state的修改会使对应证据失效。
  文档结果ledger、bit-identical merge或明确不触及验收面的变化不机械重开epoch。

## 9. 旧路径删除条件

T06只在T02–T05已从canonical checkpoint读写后删除：

- `PackageUnit`、`ServiceUnit`、`PublicationAbiUnit`及旧service assembly/bundle/index/build record模型与export。
- `artifact-identity` 中legacy service/publication/package resolver/runtime program/converter模块。
- router per-service pointer/manifest/rewrite selector/runtime target registry，runtime old graph/lazy-load/config
  registration、test-runner synthetic publication writer。
- `doc/reference/publication*.md`、`doc/architecture/release-registry.md`、runtime/router README中与权威
  设计冲突的内容。稳定规则改写到canonical reference/architecture，不保留历史兼容章节。

test-only legacy fixture只能在checker mutation sandbox中以字符串存在，必须由subject registry明确分类，
不得被production crate/module导入。

## 10. 停止条件与非目标

以下情况暂停受影响DAG分支并升级：

- 需要改变四对象、Package direct same-heap、Service boundary detached或callback lifetime。
- 需要让contract引用provider package/deployment，或让deployment重读source/AST。
- 需要保留service/version/build selector、legacy reader/writer、conversion shim、dual path或router fallback才可运行。
- 需要让不同replica加载不同partial assembly，或为某service实现RemoteBoundary/独立扩缩。
- Internals contract schema需要以package-local nominal type充当ContractTypeId，且无法按设计用显式
  contract-owned schema/wrapper转换。

非目标：历史artifact/DB数据迁移，RemoteBoundary，跨assembly调用，service级进程隔离/
独立扩缩，以及与本阶段无关的长文件全面重构。直接修改的大文件必须拆出新owner；
未触及的维护性债务记non-blocking follow-up。
