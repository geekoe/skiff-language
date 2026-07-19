# P3-F01：R01 Canonical Checkpoint Repair

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§5、§10、§11、§12、§14。
- 执行输入：`P3-T01-canonical-deployment-assembly-contract.md`、`P3-R01-canonical-contract-acceptance.md`，以及
  R01 在 integration commit `667c0622c97a30317bed566ca339c29c20302fd1` 的两个 blocking findings。
- 风险/验收组：中高风险 checker/tooling机械修复；完成后由 R01在新 exact commit复验，不新增验收 Agent。
- 当前成熟度：T01 implementation checkpoint已合流但 R01 FAIL；Wave 2继续阻塞。
- 有效证据状态：原 public-API test contract与 canonical checker ownership证据已失效；三 crate model/identity
  tests未受代码面影响，除非本任务越界修改 Rust/public model。修复 commit、checker/tests/registry或依赖变化会
  使对应复验证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent合流后重启 R01。

## DAG 与执行约束

- 依赖：R01 FAIL报告已固定；不改变 T01 schema、identity或 DAG。
- 解锁：R01复验；PASS后才解锁 T02–T05。
- branch：`codex/p3-f01-r01-checkpoint-repair`。
- worktree：`/Users/geek/workspace/skiff-p3-f01-r01-repair`。
- 五分钟内产生第一次真实代码 edit；此前不跑测试、不重做设计。若修复需要改 canonical Rust model/identity，
  立即回报范围升级，不自行修改。

## 写入范围

- `scripts/lib/crate-public-api-cli.mjs`、`scripts/lib/crate-public-api-policy.mjs` 对应的
  `scripts/tests/**` help/policy/harness契约。
- `scripts/check-artifact-identity-single-source.mjs` 的 deployment/assembly leaf owner registry与 self-test。
- 不修改 `artifact-model/**`、`artifact-identity/**`、`deployment/**`、compiler、runtime或其它无关 script。

## 完成态

1. public API characterization/harness认识 `--crate <name>`、三 crate policy与稳定 help/order；旧无参数行为仍按
   当前 canonical policy验证，不新增兼容 parser。
2. canonical registry覆盖全部 frozen deployment/assembly refs、keys、selector、link-plan、binding/template与
   ingress leaf owner，而非只登记顶层对象。
3. checker self-test至少能识别 duplicate/moved/renamed link-plan owner、link-plan legacy aggregate embedding、
   duplicate/moved/renamed service/activation template owner及第二 identity owner；不使用 broad allowlist。
4. actual checker在 production tree PASS，mutation harness在上述每类负例 FAIL。

## 唯一验证 ownership

```bash
node --test scripts/tests/crate-public-api-cli.test.mjs scripts/tests/crate-public-api-characterization.test.mjs
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-crate-public-api.mjs --crate skiff-deployment
git diff --check
```

不重跑未失效的三 crate Rust tests或完整 gate。

## 回报

提交一个 commit，回报 commit、两个 blocker的代码/负例/命令证据、未决问题及自验收矩阵：
`R01 finding | 修复证据 | 反向/负例证据 | 测试`。
