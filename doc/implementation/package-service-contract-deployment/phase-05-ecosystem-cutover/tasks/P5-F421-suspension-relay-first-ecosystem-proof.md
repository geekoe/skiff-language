# P5-F421 Suspension Relay-first fresh ecosystem proof

状态：Ready（N5；全程只读 production，唯一 tracked 写入为本任务 result）。

## 直接父节点

- `P5-F420I-final-n4-gate-result.md`
- `P5-F421A-relay-protocol-v5-receipt-oracle-result.md`
- `P5-D93-suspension-current-base-reconciliation-audit-result.md` 第 8.6 节

N4 已得到 `N4_PASS / F421_RELEASED`；F421A 已把 Relay checked-in receipt oracle 精确同步到
service protocol v5。本节点只证明当前三个 integration tree 能从空 artifact store 产生 D93
规定的 current ecosystem records，不修改任何 production、test、fixture、manifest 或 oracle。

## 精确输入

| repo | integration root | commit | tree |
| --- | --- | --- | --- |
| Skiff | `/Users/geek/workspace/skiff-phase-05-integration` | `9f39580655ecbd433235cdb7de19d823d670d4a9` | `d20cd4ccd8f11042a1f4bc6dac69d3ccda1116b9` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `baf0c907ee26e48a5fb4c153825c233bde3a6234` | `13f2f6e604fedbad80e0390e5408507430e28f8c` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `0972e65604cd4cfd45bcdb289cfe5019f57dc265` | `1849f97a1f1217b95e6e349bc529eaaf220a62f4` |

本任务 checkout 的 parent 必须精确为上表 Skiff commit/tree，且相对 parent 只能新增本任务文件。
启动时记录三个 repo 的实际 commit/tree、`git status --porcelain`、相关 lock blob 与 ancestry；任一
production tree dirty、输入不匹配或出现并发 production 写入时停止，不得自行吸收漂移。

## 写入、环境与角色边界

Skiff production write set：`∅`。Internals production write set：`∅`。
skiff-packages production write set：`∅`。

允许写入：

```text
task-owned temporary root 下的 source mirror、artifact store、assembly inputs、
command/stdout/stderr ledger、分析脚本与 receipt

Skiff：
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
P5-F421-suspension-relay-first-ecosystem-proof-result.md
```

不得修改或临时 patch source mirror 来绕过错误；mirror 必须分别由三个精确 git tree 直接生成。
不得使用 `.skiff-instance`、stable artifact root、watch registry、live service、MongoDB、旧 store、
旧 lock、旧 receipt、手工生成 artifact 或 validator waiver。不得 merge/rebase/push。

Skiff executable source自 N4 candidate 后只有 task/result 文档变化，允许复用
`/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`，但必须记录其路径，并证明
N4 executable candidate `29419bc999d441b78f1e452a454c2b24e6e30a87` 是当前 Skiff输入 ancestor，
且 candidate 到任务 checkout 没有 production diff。不得为了“fresh”复制一个可能耗尽磁盘的
Cargo target；fresh 的对象是 source mirror、artifact store和全部生成 records。

这是一次性 gate-owner 会话。可在 fresh records 已生成后，最多派两个互不重叠、只读、有明确返回物的
有界子 Agent分别核对 pair/callback 与 mapping/consumer refs；子 Agent不得再派 Agent。不得用子 Agent
重复执行 publish、assembly或竞争 verdict。

## Gate 前置预检

预检只检查命令和环境，不产生 PASS：

1. 三个输入 clean、exact且无生产写入 owner在途；
2. 当前 `node`、`cargo`、`shasum` 可用，Skiff CLI与 smoke fixture入口存在；
3. 任务临时根位于系统临时目录且有足够空间；
4. `package publish` 与 `assembly build` 的 canonical command owner仍是
   `scripts/skiff.mjs`；不得改用旧 compiler CLI或 Internals旧生成器；
5. source mirror中的全部输入文件来自对应 git archive，记录每个 mirror的 tree。

若预检发现命令 owner变化，只允许搜索当前 CLI确定等价 current命令；若需要改变 schema、manifest、
production source或公共契约，立即按下文失败交付停止。

## 唯一 fresh rebuild

建立唯一 task-owned temporary root，保存到 result提交完成；主 Agent完成宏观接收后再删除。其下至少有：

```text
source/skiff
source/internals
source/skiff-packages
artifacts
receipts/commands.jsonl
receipts/*.stdout.json
receipts/*.stderr.txt
generated-assembly/
final-receipt.json
```

使用当前 Skiff CLI、fresh source mirror和同一个空 `artifacts`根，按以下依赖顺序执行：

```text
std
  -> llm-api
  -> llm-providers
  -> Relay
  -> {Agent, http-session, track, AIHub, Account, Registry}
  -> Agine
  -> complete RuntimeAssembly
```

其中 `track` 必须在 `http-session` 后；Account也依赖`http-session`。每次 publish均使用：

```bash
node /Users/geek/workspace/skiff-phase-05-integration/scripts/skiff.mjs \
  package publish <mirrored-root> --artifact-root <fresh-artifacts> --json
```

std只能由 current smoke fixture bootstrap写入同一 fresh store：

```bash
cargo run --locked --quiet \
  --manifest-path /Users/geek/workspace/skiff-phase-05-integration/test-runner/Cargo.toml \
  --bin skiff-package-service-smoke-fixture -- \
  --bootstrap-only \
  --artifact-root <fresh-artifacts> \
  --environment p5-f421 \
  --platform-source-root <mirrored-skiff-root>
```

Relay publish成功后必须先从其 fresh `serviceDeploymentReceipt.deployment`生成只含 Relay的
`assembly.yml`，执行一次 canonical `assembly build`并完成下一节 Relay verdict；Relay未通过时
不得继续后续 ecosystem。

Relay通过后执行同一候选的 sibling wave。某个 sibling失败时，继续运行依赖已满足且与它独立的
sibling，以一次收集同一 fresh状态的生产兼容错误；不得启动依赖失败节点的 Agine，也不得修复。
全部 service成功后，用所有 fresh service deployment refs生成 complete assembly：

```bash
node /Users/geek/workspace/skiff-phase-05-integration/scripts/skiff.mjs \
  assembly build <generated-assembly-root> --artifact-root <fresh-artifacts> --json
```

每条命令必须记录 argv、cwd、输入root、起止状态、exit code及原始 stdout/stderr。JSON receipt必须
原样保存，不能只在 result抄摘要。

## Relay-first verdict

从 fresh record paths实际读取并用 canonical identity/schema reader校验，不能只检查 receipt字符串：

1. operation set精确只有：
   - `relayProxy.responsesCompleted`
   - `relayProxy.responsesCompletedResult`
2. 两个 exact concrete `PackageCallableSignature.maySuspend` 都为 `true`；
3. interface method wire递归不存在 `maySuspend`；
4. ServiceContract operation递归不存在 `maySuspend`和`cancellation`；
5. ContractOperationId精确为：
   - `skiff-contract-operation-v1:sha256:b62d89d553cc0607b2627b047d2a5ab4665c70f05f900babbce249def47099ef`
   - `skiff-contract-operation-v1:sha256:51fa082dd0d33b09f45e4900805c28801cb3108b4eac813697e66e5f8a6b007d`
6. generation精确为：
   - PackageArtifact v9；
   - Package Local ABI v7；
   - Package build v10；
   - ServiceContract v5；
   - ServiceProtocol v5；
   - ServiceDeployment v2；
   - RuntimeAssembly v2。
7. Relay PackageArtifact requirement、ServiceDeployment package binding与 Relay-only assembly
   package link的 exact package ref及`collectionNameMapping`逐跳相同。

任一项不满足即 `N5_FAIL`；不放宽 operation数量，不接受旧 generation或文本近似。

## 全生态 current records

Relay通过后，必须从所有 fresh records重新计算，不能复用 F395 的 `99 / 48 / 51`：

### Interface/concrete pairs

- hydrate每个 PackageArtifact的 FileIR；
- 以一个source type的每个`implements` interface requirement method为一个 pair；
- 排除`*.test.skiff`，保留非该后缀的`*_test_support.skiff`；
- 通过 exact FileIR/interface/type/implementation target匹配 concrete executable，读取 current
  `ExecutableIr.maySuspend`；公开 callable还要证明 Package callable summary与 concrete exact；
- receipt逐行记录 package、module/type、interface identity、method、concrete target、公开 callable
  identity（如有）及 current boolean，并汇总总数、false、true；
- 无法唯一解析的 pair是 proof失败，不能退回 source文本猜测或沿用旧计数。

### Callback records

- 遍历每个 PackageArtifact的全部`packageSchemaTypeRecords`并hydrate exact record；
- 列出每个 callback-interface的 package、stable key、type ID和operation；
- 每个 callback operation递归无provider summary / `maySuspend`；
- 若 callback count非0，用 current canonical identity命令在临时副本上证明：只改变对应 concrete
  implementor summary不会改变该 callback type ID；若 count为0，明确记录完整遍历总数与零结果，
  不伪造 mutation样本。

### Mapping 与 consumer exact refs

- 对每条 package dependency逐跳比较 PackageArtifact requirement、
  ServiceDeployment package binding、complete RuntimeAssembly package link的 exact package ref和
  `collectionNameMapping`；
- 对每条 service dependency比较 PackageArtifact service requirement、
  consumer fresh exact ServiceContract ref、provider ServiceDeployment ref及 assembly binding；
- 列出每个 consumer使用的 protocol/contract/deployment exact ref，任何旧 v4 protocol或映射漂移失败；
- 显式记录 empty mapping，不因 wire省略就跳过比较。

## 必须执行的负例与反向搜索

只在 task-owned临时副本上做变异，并调用 current canonical reader/validator：

1. interface/callback/ServiceContract分别注入已删除的`maySuspend`，ServiceContract再注入
   `cancellation`，均须严格拒绝；
2. 把一个 current prefix降为对应旧 PackageArtifact v8、Local ABI v6、build v9或protocol v4，
   均须拒绝；
3. Relay删除/增加 operation、把 concrete summary改为false、改变 operation ID，均须被 proof checker
   检出；
4. 对一个非空或显式空 mapping制造 requirement/binding/link drift，须被 current
   deployment/assembly validator拒绝；
5. 对一个 consumer改成旧 protocol ref，须被 current resolver/loader拒绝；
6. 反向搜索全部 fresh JSON，旧字段只允许存在于上述故意变异文件；canonical records中为0。

若某负例没有现成 canonical CLI入口，可使用 current Rust/Node validator的已有聚焦测试或写
task-owned临时调用器；不得新增 tracked test或把纯字符串比较冒充 schema拒绝。

## Receipt 与 verdict

`final-receipt.json`至少包含：

- schema/version与最终 `N5_PASS`或`N5_FAIL`；
- 三个 input commit/tree、lock blob、mirror tree、Skiff N4 ancestry与 production diff；
- 每步完整命令及stdout receipt文件；
- 每个 artifact/contract/deployment/assembly record path、SHA-256、schema与identity prefix；
- Relay operation、concrete summary、interface wire、contract field、operation ID及mapping断言；
- 全量 pair列表和 current分类；
- 全量 callback record列表；
- package/service mapping与consumer exact refs；
- 每个负例的 mutation、canonical validator与拒绝证据；
- stable/live/old store/waiver均未使用，三个 production tree在任务结束时仍clean。

全部条款通过才写：

```text
N5_PASS
PHASE_05_ECOSYSTEM_PROOF_COMPLETE
```

若任一 production source不能由 current CLI发布、任一 required record缺失，或继续完成需要修改
manifest/source/compiler/runtime/router，则停止并写：

```text
TASK_SCOPE_EXPANDED
N5_FAIL
```

失败 result必须记录最后成功阶段、同一 sibling wave的全部独立错误、首个 canonical命令、
完整 stderr、受遮挡节点以及最小 successor owner；不得自行修 source、补 compatibility或提出
未经证据支持的宽泛重构。

只提交 Skiff result一个 commit，worktree最终 clean。返回 result commit/tree、临时根路径、
实际生成记录数、验证矩阵和未决 blocker；不得自行承接 successor。
