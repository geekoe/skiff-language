# P5-F445H-I7-T0 Internals isolated gate tooling

状态：`IN_PROGRESS`。

本节点是 I7 DAG 的 T0 shared test-tooling implementation checkpoint。直接父节点为：

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`
- `P5-F445H-I6K-R4-independent-current-scope-reacceptance-result.md`

父节点继续追溯到唯一权威设计
`doc/architecture/package-service-contract-deployment.md`。I6 R4 已满足
`PASS / I6_ACCEPTED = YES / I7_UNBLOCKED = YES`。T0 完成只解除 Internals C/A；
最终 J 仍等待 `T0 + C + A + U`。

## 1. Exact inputs and owners

| 项 | 值 |
| --- | --- |
| Skiff source | `54fb087f122c53aed5c017260c7bca43e2b54404` / `008d3a05927cdf845004db980d1b46de263612be` |
| Internals baseline | `19d41001f048efc0b70e13c21d105a855ddd86e2` / `15c48e07cc3d51794269719c606c87169bd0ee72` |
| Internals integration branch | `codex/package-service-phase-05` |
| implementation branch | `codex/p5-f445h-i7-t0-tooling` |
| implementation worktree | `/Users/geek/workspace/internals-p5-f445h-i7-t0-tooling` |
| Internals integration owner | `/root/phase05_internals_integration_steward` |
| Skiff task/result owner | `/root/phase05_integration_steward` |

Skiff source 相对 I6 frozen candidate 的相关 runtime、CLI 与 test-runner没有 diff。

## 2. Preflight RED and current owner

Internals baseline 已拒绝旧 `--packages-dir` / `--service-artifact-root` 并使用 temp artifact
root，但 canonical refactor 丢失 target-specific actual-test语义：

- `check-isolated-service-graph.mjs` 与 `test-isolated-service.mjs` 校验 target 后仍无条件运行完整
  `{codex-relay,aihub,agine,account}` publish + assembly；
- `test-isolated-service.mjs` 完全不 spawn `skiff test`，所以 `.test.skiff` 未执行；
- 因此 C 会被 A/Account 遮挡，C/A 不能隔离并行。

真实 owner / expected write set：

```text
scripts/isolated-service-graph.mjs
scripts/check-isolated-service-graph.mjs
scripts/test-isolated-service.mjs
scripts/isolated-service-graph.test.mjs
scripts/test-isolated-service.test.mjs
scripts/prepare-canonical-assembly.mjs
scripts/prepare-canonical-assembly.test.mjs
```

后两个 helper 只允许机械扩展并复用既有 bootstrap/authoring owner，不得复制它。

## 3. Required behavior

`check` 必须按 target transitive closure发布并在包含 target 后 resolve assembly。`test` 必须在
同一个 owned temp artifact root 中只准备 target package dependencies 与 service providers，再精确
spawn frozen current CLI：

```bash
node <skiff>/scripts/skiff.mjs test <targetRoot> \
  --artifact-root <root> \
  --deny-skips \
  --require-tests
```

non-live runtime/Mongo由 Skiff CLI 自身 managed isolation拥有。wrapper不得读取 `27017`、
`.skiff-instance`、reload URL 或 legacy flags。

目标 topology：

```text
codex   -> {codex}
aihub   -> {codex, aihub}
agine   -> {codex, aihub, agine}
account -> {account}
```

Account保持独立 target，不混入 Agine 三 service topology。target缺失/未知、零 test selection、
legacy flags、stable path或shared Mongo必须 fail closed。保留三个精确 repo root/provenance 和一个
shared temp artifact root；signal/error cleanup 继续只有一个 owner。

## 4. Write and permission boundaries

禁止修改任何 service/client/Host production或fixture、canonical-source public provenance
contract、Skiff/packages、stable store/watch/reload、network、Mongo、OAuth、browser或 live状态。

若完成需要改变 business service semantics/public contract/canonical three-repo provenance、复制
bootstrap owner、触碰 service fixture或外部状态，返回 `TASK_SCOPE_EXPANDED`。

## 5. Verification owner

本 leaf 是以下证据的唯一 owner：

```bash
node --test \
  scripts/isolated-service-graph.test.mjs \
  scripts/test-isolated-service.test.mjs \
  scripts/prepare-canonical-assembly.test.mjs

node --check <all touched production/test .mjs files>
git diff --check
```

fake runner 必须精确断言 target topology、package/service publish顺序、shared artifact root、
actual test invocation 与 negative mutations。T0 不运行真实 service matrix、Cargo/Skiff CLI、
stable/network/Mongo；真实 matrix 的唯一 owner 是 J/后续 service leaves。

## 6. Candidate and handoff

本节点是低风险 shared test-tooling implementation checkpoint；完成后只解除 C/A，不升级整个 I7。
证据会因 Skiff test CLI/isolation contract、Internals graph/provenance/helper、任一 touched
script/test或精确 repo identity变化而失效。

Internals implementation+tests使用一个 commit。验证后开发 Agent把精确 commit/tree与结构化
result交给 Skiff文档 owner；Skiff owner另提交 result。开发 Agent不得 merge、rebase或push。

