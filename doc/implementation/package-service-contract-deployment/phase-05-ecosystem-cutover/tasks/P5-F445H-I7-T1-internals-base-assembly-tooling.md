# P5-F445H-I7-T1 Internals dependency base-assembly tooling

状态：`READY / EXECUTABLE`。

本节点是 I7 DAG 的 T1 shared-tooling correction。直接父节点为：

- `P5-F445H-I7-T0-internals-isolated-gate-tooling-result.md`；
- `P5-F445H-I7-C1-codex-relay-provider-checkpoint-result.md`。

父链经 I7R与I6K-R4追溯到唯一权威设计
`doc/architecture/package-service-contract-deployment.md`。T1完成后解除C的isolated blocker，并同步
解除A对同一shared wrapper contract的blocker；不代表C、A、I7或J完成。

## 1. Frozen inputs and owners

| 项 | 值 |
| --- | --- |
| Internals baseline | `c9152c7745769bb995ac7265322db678851883ee` / `9e846fcdcbb12f539be822276395deaff4abbe7f` |
| implementation branch | `codex/p5-f445h-i7-t1-base-assembly` |
| implementation worktree | `/Users/geek/workspace/internals-p5-f445h-i7-t1-base-assembly` |
| Internals integration owner | `/root/phase05_internals_integration_steward` |
| Skiff task/result owner | `/root/phase05_integration_steward` |

Internals leaf只提交scripts/tests。Phase 05 task/result文档由Skiff owner独立提交。

## 2. Frozen RED and classification

baseline 的 `scripts/test-isolated-service.mjs` 使用：

```text
includeTarget: false
resolveAssembly: false
```

它会发布service dependencies，却不生成 RuntimeAssembly；随后 `skiff test` invocation也没有
`--base-assembly`。current Skiff runner对带runtime service requirement的target要求exactly one
base assembly，因此AIHub必然得到：

```text
found 0
```

这是T0 shared tooling contract gap，不是A/C service、fixture或Skiff runner语义缺陷。禁止通过修改
AIHub/Relay manifests、service source或runner语义掩盖。

## 3. Required behavior

test workflow必须在同一个owned temporary artifact root中：

1. 发布target transitive packages；
2. 只发布service dependencies，target service仍不发布；
3. 从dependency deployments构建唯一current RuntimeAssembly；
4. 提取canonical nested receipt中的assembly identity；
5. 对一次精确invocation传递：

```text
skiff test <targetRoot>
  --artifact-root <owned-root>
  --base-assembly <dependency-assembly-identity>
  --deny-skips
  --require-tests
```

`--base-assembly`必须精确出现一次。identity必须来自：

```text
runtimeAssemblyReceipt.assembly.assemblyIdentity
```

并匹配current canonical `skiff-runtime-assembly-v2` identity。

AIHub dependency assembly的`rootDeployments`必须只有Relay；不得泄漏AIHub target、Agine sibling或
Account service。没有service dependencies的target不得伪造Skiff-invalid empty assembly，也不得
收到无关assembly。

以下情况必须fail closed：

- nested receipt identity缺失、旧shape或格式非法；
- zero/multiple `--base-assembly`；
- identity来自无关assembly；
- path逃出owned artifact root；
- legacy/stable/shared-Mongo mutation；
- target、sibling或Account泄漏到dependency assembly。

不得复制bootstrap或assembly authoring owner。

## 4. Write ownership

production/helper owners：

```text
scripts/prepare-canonical-assembly.mjs
scripts/test-isolated-service.mjs
```

机械tests：

```text
scripts/prepare-canonical-assembly.test.mjs
scripts/test-isolated-service.test.mjs
```

`prepare-canonical-assembly.mjs` 必须复用唯一dependency assembly authoring/receipt owner；
`test-isolated-service.mjs` 只负责生成exact test invocation。若无需触碰其它T0 files，不得扩写。

禁止修改：

- service/client/Host source或fixtures；
- public provenance；
- Skiff或official packages；
- stable/watch/reload、live、external network、Mongo、OAuth或browser。

## 5. Verification owner

T1唯一验证矩阵：

```bash
node --test \
  scripts/isolated-service-graph.test.mjs \
  scripts/test-isolated-service.test.mjs \
  scripts/prepare-canonical-assembly.test.mjs

node --check scripts/prepare-canonical-assembly.mjs
node --check scripts/prepare-canonical-assembly.test.mjs
node --check scripts/test-isolated-service.mjs
node --check scripts/test-isolated-service.test.mjs

git diff --check
```

fake spawn/unit必须证明：

- exactly one base-assembly flag；
- identity来自dependency assembly nested receipt；
- AIHub只包含Relay deployment；
- target/sibling/Account无泄漏；
- 单一artifact/Cargo root；
- stripped stable env；
- `--deny-skips`与`--require-tests`保留；
- missing/invalid/duplicate/unrelated identity fail closed。

禁止运行真实service matrix、Skiff Cargo、stable/live/network或Mongo。

## 6. Stop conditions and handoff

若完成需要business fixture/public contract、Skiff runner语义、新bootstrap owner、外部状态或兄弟
shared owner，返回 `TASK_SCOPE_EXPANDED`。否则机械helper/test closure可在上述四文件内自主完成。

结果必须记录：

- implementation commit/tree；
- actual write set；
- test-first RED与最终evidence matrix；
- `T1_COMPLETE`；
- `C_ISOLATED_UNBLOCKED`；
- `A_ISOLATED_UNBLOCKED`；
- cleanup、residual risk与evidence invalidation。

这是低风险shared tooling implementation checkpoint。scripts/tests、Skiff
test/base-assembly CLI、receipt schema、canonical target graph/provenance、repo identity或temp/env
ownership变化会使证据失效。

leaf不得自行merge、rebase、push或清理一级worktree。Internals实现交integration steward；结构化
result交Skiff docs owner。
