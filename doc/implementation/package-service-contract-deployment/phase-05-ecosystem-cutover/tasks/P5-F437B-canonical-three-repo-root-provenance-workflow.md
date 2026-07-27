# P5-F437B Canonical three-repo root/provenance workflow

状态：Ready。中风险Internals tooling checkpoint；与F438审计并行。

## 直接父节点

- `P5-F437A-aihub-canonical-publish-path-closure-audit-result.md`

父result已冻结完整dependency DAG、canonical顺序、当前root omission、wrong-checkout风险、receipt缺口与
禁止fallback。该引用链继续到唯一权威设计。启动时只读本任务，需要依据时沿父节点读取。

## 精确输入与DAG

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff（只读toolchain） | `64a0ab4ec85d25899dc8563ac6d647edad8ed23e` | `562adcfc8baa595969a4dd1ccd2e67c4053814b9` |
| Internals（写入） | `066b5135a8e06f87acfd614e408e05b35453f4eb` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` |
| skiff-packages（只读source） | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

```text
F437B workflow/provenance checkpoint
  -> Agine authoring/config与selector repairs合流
  -> cheap full canonical publish/assembly probe
```

当前候选不是稳定状态；本任务不修Agine source/config/service.yml或assembly selector collision。

## 写入范围

只允许修改：

- `scripts/prepare-canonical-assembly.mjs`
- `scripts/check-isolated-service-graph.mjs`
- `scripts/test-isolated-service.mjs`
- `scripts/isolated-service-graph.mjs`（仅确需承载canonical input/provenance invariant时）
- `scripts/prepare-canonical-assembly.test.mjs`
- `scripts/isolated-service-graph.test.mjs`
- `scripts/test-isolated-service.test.mjs`
- `AGENTS.md`（只同步canonical linked-worktree所需的两个显式root）
- 如职责确实独立，可在`scripts/`新增一个只负责canonical source root/provenance的小模块及direct test
- 本leaf result

禁止修改service/package source、manifest、package.json、skiff-packages、Skiff production/test、
stable/local config或其它workflow。若正确实现需要改调用方package scripts或公共CLI，返回
`TASK_SCOPE_EXPANDED`，不要越界。

## 必须实现

1. Exported canonical workflow函数显式接收三个root：
   `internalsRoot`、`skiffRoot`、`skiffPackagesRoot`。Executable入口必须要求绝对
   `SKIFF_ROOT`与`SKIFF_PACKAGES_ROOT`；缺失时fail closed，不猜sibling、不回退main/stable。
2. Canonical package顺序精确为：

   ```text
   <skiff>/std
   <internals>/packages/llm-api
   <internals>/packages/llm-providers
   <internals>/packages/agent
   <skiff-packages>/http-session
   <skiff-packages>/track
   ```

   service顺序保持Codex Relay → AIHub → Agine → Account，最后构建唯一四root assembly。
3. 在任何authoring命令前验证：
   - 三root都是绝对、存在、各自git top-level与传入root一致；
   - 能读取精确commit与tree；
   - 每个canonical coordinate解析到预期root下唯一source目录；
   - `http-session`/`track`不能来自Internals、Skiff或其它猜测路径；
   - duplicate、缺root、wrong checkout/mapping与顺序错误fail closed。
4. Workflow result/receipt必须记录三仓root、commit、tree及每个package/service
   coordinate → source root映射，使后续gate能判断实际使用的checkout。该metadata不改变Skiff artifact
   schema或public CLI。
5. `--list --fixture-only`输出包含两个official roots、三仓provenance和完整确定性顺序；list也必须对缺失
   env/wrong root fail closed，不能只在真实执行时校验。
6. 保持单一temporary artifact root、std bootstrap owner、typed record/pointer resolution、receipt完整性、
   signal cleanup与linked-worktree mutation guard现有语义。
7. Direct tests必须至少覆盖：
   - exact六package/fourservice顺序；
   - omitted `SKIFF_PACKAGES_ROOT`；
   - nonexistent/non-absolute root；
   - wrong git top-level或coordinate/root mapping；
   - duplicate/missing official root；
   - provenance commit/tree记录；
   - partial package/service receipt仍fail closed；
   - 命令不含legacy source store/stable fallback。
8. 反向搜索确认三个executable入口不再含`join(dirname(internalsRoot), 'skiff')`等root fallback，
   且所有`runCanonicalFixtureWorkflow` / `canonicalFixtureInputs` caller传递
   `skiffPackagesRoot`。

## 验证

本Agent是以下聚焦证据的唯一owner：

```bash
node --test scripts/prepare-canonical-assembly.test.mjs \
  scripts/isolated-service-graph.test.mjs \
  scripts/test-isolated-service.test.mjs
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f437b-canonical-roots \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-p5-f437b-canonical-roots \
  node scripts/prepare-canonical-assembly.mjs --list --fixture-only
node --check scripts/prepare-canonical-assembly.mjs
node --check scripts/check-isolated-service-graph.mjs
node --check scripts/test-isolated-service.mjs
git diff --check
```

List命令只枚举并验证source/provenance，不运行publish。完整canonical authoring由后续combined owner在
Agine与selector blocker合流后执行，本任务不得提前重复。

## Worktree与交付

- Internals worktree：`/Users/geek/workspace/internals-p5-f437b-canonical-roots`
- Skiff result worktree：`/Users/geek/workspace/skiff-p5-f437b-canonical-roots`
- skiff-packages只读 worktree：`/Users/geek/workspace/skiff-packages-p5-f437b-canonical-roots`
- 分支：`codex/p5-f437b-canonical-roots`

启动后5分钟内完成第一次实际代码修改，或报告具体未知量。Internals提交implementation；随后在Skiff
新增并提交`P5-F437B-canonical-three-repo-root-provenance-workflow-result.md`。返回两个commit/tree、
验证矩阵、反向搜索与三个clean状态。不得merge、rebase、push、stable/live或承接后续combined。
