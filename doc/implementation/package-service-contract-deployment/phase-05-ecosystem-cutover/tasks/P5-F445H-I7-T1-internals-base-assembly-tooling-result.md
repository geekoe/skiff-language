# P5-F445H-I7-T1 Internals dependency base-assembly tooling result

状态：

```text
PASS
T1_COMPLETE = YES
BLOCKING_ISSUES = 0
TASK_SCOPE_EXPANDED = NO
C_ISOLATED_UNBLOCKED = YES
A_ISOLATED_UNBLOCKED = YES
J_T1_PREREQUISITE_SATISFIED = YES
```

T1已修复T0 shared isolated wrapper的dependency base-assembly缺口。带service dependencies的target
现在会在同一owned isolation/artifact root中，为dependency deployments构造唯一current
RuntimeAssembly，从canonical nested receipt提取identity，并对精确一次 `skiff test` invocation
传递精确一次 `--base-assembly`。

本结果解除C/A的shared isolated blocker，并满足J的T1前置；不代表C、A、I7或J完成。真实C/A
service matrices仍由各自下游owner运行。

## 1. Exact identities and integration

| 项 | 值 |
| --- | --- |
| Skiff task commit/tree | `3a5b9c9fb83e7c58fb36434dd52c56e170f1d6e9` / `3e7fddad25d34bbe64f596906646af67f45ad714` |
| Internals baseline | `c9152c7745769bb995ac7265322db678851883ee` / `9e846fcdcbb12f539be822276395deaff4abbe7f` |
| implementation commit/tree | `03025bb315ed083fa3f270f0b7c49024c7d7ed56` / `aa219a1f84ec7dfb651d90cb0dc8e9e4c3799c89` |
| Internals integration merge/tree | `c069dd2cec0db25c965807d320fa20e6cd76178d` / `2778df38c5f0247c16c2992c6b473e0fd7144ea9` |
| leaf branch | `codex/p5-f445h-i7-t1-base-assembly` |
| leaf worktree | `/Users/geek/workspace/internals-p5-f445h-i7-t1-base-assembly` |

implementation commit的parent精确为Internals baseline。Internals integration steward已完成合流：
integration status clean、`git diff --check` PASS；leaf worktree与branch已删除。leaf没有自行
merge、rebase或push。

## 2. RED classification

T0 wrapper原来使用：

```text
includeTarget: false
resolveAssembly: false
```

它发布providers但不生成 RuntimeAssembly，test invocation也不传 `--base-assembly`。current Skiff
runner对runtime service requirements要求exactly one base assembly，因此AIHub必然 `found 0`。

test-first RED精确暴露：

- dependency assembly helper export缺失；
- invocation args缺少base identity；
- dependency-only RuntimeAssembly没有被author。

这是shared T0 tooling gap，不是C/A service、fixture或Skiff runner design defect。

## 3. Actual write set

实现写集精确为四个授权文件：

```text
scripts/prepare-canonical-assembly.mjs
scripts/prepare-canonical-assembly.test.mjs
scripts/test-isolated-service.mjs
scripts/test-isolated-service.test.mjs
```

没有service/client/Host、fixture、public provenance、Skiff、official packages或其它T0 file写入。

## 4. Delivered behavior

对有service dependencies的target：

1. target transitive packages与仅service dependencies在同一owned artifact root准备；
2. target service仍不发布；
3. dependency deployments author唯一current RuntimeAssembly；
4. helper要求精确一个nested
   `runtimeAssemblyReceipt.assembly.assemblyIdentity`；
5. identity必须匹配current canonical `skiff-runtime-assembly-v2`；
6. exact invocation传递一次
   `--base-assembly <dependency-assembly-identity>`；
7. `--artifact-root`、`--deny-skips`、`--require-tests`保持。

AIHub assembly的exact `rootDeployments`只包含Relay；没有AIHub target、Agine sibling或Account
泄漏。没有service dependencies的target不会伪造Skiff-invalid empty assembly，也不会收到无关
assembly。

missing、old-shape、illegal、multiple或unrelated identity/flag全部fail closed。wrapper继续复用
canonical bootstrap/assembly owner，不复制authoring逻辑。

## 5. Evidence ledger

| 检查 | 结果 |
| --- | --- |
| `node --test scripts/isolated-service-graph.test.mjs scripts/test-isolated-service.test.mjs scripts/prepare-canonical-assembly.test.mjs` | PASS `25/25` |
| `node --check` 四个touched files | PASS `4/4` |
| implementation `git diff --check` | PASS |
| integration `git diff --check` | PASS |

fake runner/unit精确证明：

- dependency manifest与nested identity transfer；
- exactly one base-assembly flag；
- AIHub only-Relay deployment；
- target/sibling/Account无泄漏；
- shared artifact/Cargo roots；
- stable env被剥离；
- deny-skips/require-tests保留；
- missing/invalid/duplicate/unrelated base全部拒绝。

## 6. Isolation, residual risk and invalidation

T1没有运行真实service matrix、Skiff Cargo、stable/watch/reload、live、network、Mongo、OAuth或
browser，没有push。真实C/A matrix属于下游owners。

以下变化会使证据失效：

- 四个scripts/tests任一变化；
- Skiff test/base-assembly CLI或RuntimeAssembly receipt v2 schema变化；
- canonical graph/provenance变化；
- Internals/Skiff/package frozen identity变化；
- temporary artifact/Cargo root或environment ownership变化。

```text
T1_COMPLETE = YES
C_ISOLATED_UNBLOCKED = YES
A_ISOLATED_UNBLOCKED = YES
J_T1_PREREQUISITE_SATISFIED = YES
```

J仍等待其它精确父节点。
