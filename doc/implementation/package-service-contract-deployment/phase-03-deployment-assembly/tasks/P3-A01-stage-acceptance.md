# P3-A01：Independent Stage Acceptance

## 角色与输入

未参与 Phase 03开发和 integration的独立只读验收 Agent。不得修改文件、创建 commit或替开发 Agent解释实现。

完整阅读唯一权威设计 `doc/architecture/package-service-contract-deployment.md`（重点 §2、§5、§9、§10、
§11、§12、§14）、Phase 03 overview/plan/tasks/result，并在 T09记录的 exact clean commit上核验 production
代码与证据。风险组为阶段级独立验收；已有昂贵 gate在同一 commit有效时不机械重跑。任何 production/public
surface、依赖、checker、fixture、配置或 gate环境变化都按影响面使验收证据失效。

## 必验条款

1. 四对象 owner与 identity严格分离；deployment/assembly是 source-free typed pipeline，无 legacy aggregate、
   adapter、dual-read或 fallback。
2. deployment operation mapping显式且完整；Available/descriptor/ContractTypeId/value plan/effect与所有
   implementation requirement均 fail closed验证。
3. package binding以 caller build + alias选择 exact build/version/local ABI；service requirement不携带 provider
   build/revision/route，assembly内恰好一个本地 provider。
4. A↔B service cycle和package closure真实闭合；零/多 provider、remote-only、ABI/protocol mismatch失败。
5. shared PackageBuildId只共享 immutable code/image；service/config/state/resource/callback mutable owner按
   activation分离，service slot key包含 caller build。
6. service call仍走 activation-relative `InProcessBoundary` plan，不被 linker patch为 package direct call或
   provider executable；Phase 04 execution尚未被偷做。
7. RuntimeAssembly canonical link plan/templates/global ingress/empty assembly及 identity inclusion/exclusion符合设计；
   secret material、path/display/replica state不进入 identity。
8. loader/linker/admission在请求前验证 exact refs、File IR/resource/link plan/template tamper；raw JSON/source/
   display无法成为 semantic owner。
9. admitted active assembly保留 immutable canonical ServiceContract store；ref + operation ID可取得 descriptor/value
   plan，template不复制 owner，请求路径不重载 contract。
10. whole-assembly candidate一次性 admit/atomic swap；失败保留旧 active；request path零 artifact I/O/lazy load；
   health能观察 identity与最后状态。
11. structure checker与 self-test能发现旧 DTO、raw/display/source linking、lazy load、第二 owner、改名/移动/
    duplicate/test-only伪例外；无 broad allowlist或 ledger。
12. T09需求→代码→测试证据对应同一 stable commit；高风险边界有真实 producer/consumer E2E与负例，不是只靠
    serde fixture。
13. authoring/registry/router/test-runner/telemetry、ActivationContext execution、async/stream/callback/cancel、
    RemoteBoundary确实留在后续阶段，未用临时兼容实现填洞。

## 输出

第一行必须为 `PASS` 或 `FAIL`。`FAIL` 列 blocking issue、设计/任务证据、production代码证据、影响、建议
owner及使哪些 gate失效；另列 non-blocking follow-up、已运行聚焦命令和未覆盖动态风险。
