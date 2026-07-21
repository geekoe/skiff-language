# Phase 05：Ecosystem Cutover 实现计划

状态：active；P5-R01 已在 `c168b1dc` / tree `961998ac` PASS。T02/T03/T04已合流、T05在途；
R02预审在`b47ddf7`发现T03/T04真实wire/startup/request/pin/storage双owner断链。F03A已合流，R02A在
`a7566bb`因canonical request optional fields的TS/Rust接受集合不一致而FAIL；D03有界审计与F03A1 repair在途。

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
  T05 test-runner / package-test / fixtures              ─┘

  R02 pre-review@b47ddf7 findings
    └─► F03A shared binary/request/store seam ─► R02A FAIL@a7566bb
          └─► D03 canonical optional-field parity audit ─► F03A1 bounded repair ─► R02A narrow reverify
                ├─► F03B unified Router endpoint/store/pin ─┐
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
| F03A | [Router/runtime shared seam](tasks/P5-F03A-router-runtime-shared-seam.md) | R02 pre-review findings | 高；binary/header/store checkpoint |
| R02A | [Router/runtime seam acceptance](tasks/P5-R02A-router-runtime-seam-acceptance.md) | F03A exact commit | 独立只读；不作R02 verdict |
| D03 | [Canonical request optional parity audit](tasks/P5-D03-canonical-request-optional-parity-audit.md) | R02A FAIL at `a7566bb` | 独立只读；冻结完整字段矩阵 |
| F03A1 | [Canonical request optional parity repair](tasks/P5-F03A1-canonical-request-optional-parity-repair.md) | D03 complete | 高；共享request codec/corpus窄修复 |
| F03B | [Router integration repair](tasks/P5-F03B-router-integration-repair.md) | R02A PASS | 高；unified endpoint/store/pin |
| F03C | [Runtime integration repair](tasks/P5-F03C-runtime-integration-repair.md) | R02A PASS | 高；startup/admission/pin |
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
  runtime harness；不改runtime host production、router或tooling production。
- F03A在R02预审后串行独占Router/Runtime shared wire、compiler internal canonical-store adapter与cross-language
  fixture。R02A首次FAIL后，D03只读穷举canonical request所有optional/nested字段的两端接受集合；F03A1只改
  request shared codec、直接tests与同一cross-language corpus，不回改已PASS的activation/store，也不实现consumer。
  R02A窄复验PASS后F03B只写Router consumer，F03C只写Runtime consumer。两者不得回改shared seam；所有
  repair与T05合流后才刷新Cargo.lock并运行I02，避免单侧mock再次冒充interop证据。
- T06在R02后独占 `artifact-model`/`artifact-identity`/runtime linked/eval的旧module删除，canonical
  ecosystem checker/verify接线、`cross-system-fixtures/**`及旧reference/architecture/runtime/router文档。
  不修改已通过R02的consumer语义。
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
