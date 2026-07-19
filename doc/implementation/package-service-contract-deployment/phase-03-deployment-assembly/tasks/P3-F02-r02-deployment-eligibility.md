# P3-F02：R02 Deployment Typed Eligibility Repair

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2列表4、§5、§8、§9、§14。
- 执行输入：T02合同、R02合同，以及 R02在 exact commit
  `46d7b6f60aa19b2ea133a56772a50adb0ffd2726` 的 DEPLOYMENT blocker。
- 风险/验收组：高风险 deployment trust-boundary定点修复；完成后由 R02只复验 DEPLOYMENT verdict。
- 当前成熟度：T02已合流但 R02整体 FAIL；T03 ASSEMBLY verdict仍 PASS，T06继续阻塞。
- 有效证据状态：T02 projection eligibility与其 8 个测试失效；T03 assembly及 R01证据不失效。projection代码/
  typed artifact依赖/fixture/tests变化会使修复证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent合流后重启 R02。

## DAG 与执行约束

- 依赖：R02 finding已固定；不改变 frozen schema/identity、T03 resolver或 runtime。
- 解锁：R02 DEPLOYMENT复验；与既有 ASSEMBLY PASS共同解锁 T06。
- branch：`codex/p3-f02-deployment-eligibility`。
- worktree：`/Users/geek/workspace/skiff-p3-f02-deployment-eligibility`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计。若必须修改 canonical model/identity或 compiler，
  回报 checkpoint amendment，不越界实现。

## 写入范围

独占 `deployment/src/projection/**`。不得修改 `artifact-model/**`、`artifact-identity/**`、assembly、runtime、
compiler或 checker。

## 完成态

1. deployment不信任 `BoundaryCallableProjection::Available` 标签本身；用 PackageArtifact中已有 typed
   `CallableSemanticFacts`、provenance、resolved call-target与 implementation requirements独立校验该 callable
   确实满足 contract公开 effect guarantee。
2. caller-reachable mutation、return/throw alias、escape/capture、same-heap identity、unsupported callback/native
   requirement、unknown provenance、unknown call/effect或 unknown resolved target，只要与 contract guarantee/
   boundary语义不相容就 fail closed。
3. 即使 attacker同步修改 semantic facts、`completeMayEffects`、implementation requirements并重新 assign合法
   PackageArtifactIdentity，也不能绕过 deployment trust boundary。
4. 验证只读取 typed PackageArtifact/ServiceContract，不依赖 compiler/source/lowering，不重新解析 AST或
   File IR opaque signature。
5. 增加 mutation测试：以合法 Available fixture为基线，分别注入 unsafe may-effect、Unknown provenance、
   Unknown resolved target并重新 assign identity，projection均给稳定结构化错误；安全 baseline仍成功。

## 唯一验证 ownership

```bash
cargo test -p skiff-deployment projection
rg -n '\b(compiler|source|lowering|PackageUnit|ServiceUnit|serviceAssembly)\b' deployment/src/projection
git diff --check
```

不重跑 assembly、R01、runtime或完整 gate。

## 回报

提交一个 commit，回报 commit、R02反例如何被拒绝、mutation矩阵、命令与自验收矩阵：
`R02 finding | typed validation证据 | mutation/反向证据 | 测试`。
