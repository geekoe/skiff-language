# P5-F437A AIHub canonical publish path closure audit

状态：Ready。高风险、只读、收敛熔断后的路径闭合审计。

## 直接父节点

- `P5-F436A-aihub-interface-self-receivers-result.md`
- `P5-F435A-generic-call-concrete-return-typing-result.md`
- `P5-F434A-aihub-correlation-free-http-combined-result.md`
- `P5-F264-agent-public-schema-closure.md`

F434A、F435A 与 F436A 已在同一 canonical isolated publish 路径连续暴露 generic return、
interface receiver 与 official package pointer 三层 blocker。按收敛规则，在下一次完整 combined
前必须一次闭合剩余明确 production 范围，不能继续逐首错修补。

## 精确输入

| repo | commit | tree |
| --- | --- | --- |
| Skiff integration | `1b4fb5b81049ef310e539f2888a96237895012d6` | `65b6edb922448cd32174b673bf41717d9caa4f08` |
| Internals integration | `066b5135a8e06f87acfd614e408e05b35453f4eb` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` |
| skiff-packages integration | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

当前 `check-isolated-service-graph.mjs agine.ai/aihub` 实际运行全 canonical fixture。fixture
发布 std、三个 Internals package 和四个 service package，却未发布后两者依赖的
`skiff.run/http-session`/`skiff.run/track`，因此最新首错是缺少
`skiff.run/http-session@1.0.0` pointer。F264 还记录过 official package 自身 API closure
缺口；必须以当前三个 integration 输入重新判定，不能沿用历史结论。

## 读取、临时状态与写入范围

允许读取：

- Internals `scripts/**` canonical workflow、四个 service root及其完整 package/service dependency
  declarations、相关 workflow tests；
- skiff-packages 中依赖闭包涉及的 official package roots、tests 与 manifests；
- Skiff package publish/resolve、bootstrap、artifact pointer 和 assembly tooling owner；
- 直接父 result 及其引用链。

允许在一次显式临时目录中生成隔离 artifact root、Cargo target、npm cache 与日志；结束前删除。
不得修改 Internals、skiff-packages 或 Skiff production/test/fixture。唯一 repo 写入是本 leaf
result。

## 必须回答

1. 从四个 service roots 与所有 package manifests 计算完整有向 dependency closure和唯一合法
   topological publish order；列出当前 fixture 缺少、重复或顺序错误的每个 root。
2. 精确解释为什么以 AIHub 为 target 的 type-check 会进入 Agine/account dependency blocker：
   target 参数、full fixture、receipt与 assembly owner分别在哪里。
3. 判断 official package root 应如何显式进入 linked-worktree canonical workflow：
   - path/env/CLI 的现有 owner；
   - 不得读取 stable store、source symlink或主 worktree artifact；
   - 三 repo exact checkout 如何成为可复现输入；
   - 不得靠猜 sibling path 或兼容 fallback 隐式补包。
4. 在临时 artifact root 按完整 dependency order做一次有界 crossing probe：
   - 只运行为枚举 publish/assembly 剩余 blocker所需的 package/service authoring；
   - 每个失败记录精确 stage、root、诊断、owner与被遮挡下游；
   - 可用直接静态/聚焦 probe检查被首错遮挡的其它 root，但不得修改 source 伪造通过；
   - 不运行 HTTP stream combined、service E2E、live provider 或 stable。
5. 重新核对当前 `http-session`、`track` 及其它 official package：
   Package API closure、config/state requirement、source typing、publication pointer 与其 consumer
   的 exact version/alias。区分历史已修、当前真实 blocker和纯 fixture omission。
6. 检查 canonical workflow 的 tests/receipts 是否能发现 dependency root 遗漏、发布顺序错误、
   wrong checkout 与 partial receipts；指出需要新增或加强的最小负例。
7. 输出一个批量 repair DAG：
   - shared workflow/root discovery checkpoint；
   - 若存在，按 repo/owner拆分的 official package source repairs；
   - 合流后的便宜 combined integration probe；
   - 只有 probe 通过后才重跑 AIHub full combined。
8. 若 root 输入契约或三 repo checkout选择涉及尚未冻结的公共行为，返回
   `TASK_SCOPE_EXPANDED`并给出最小决策问题；不要自行引入 fallback。

## 交付

- Skiff worktree：`/Users/geek/workspace/skiff-p5-f437a-aihub-publish-audit`
- Internals worktree：`/Users/geek/workspace/internals-p5-f437a-aihub-publish-audit`
- skiff-packages worktree：`/Users/geek/workspace/skiff-packages-p5-f437a-aihub-publish-audit`
- 分支：`codex/p5-f437a-aihub-publish-audit`

新增并提交
`P5-F437A-aihub-canonical-publish-path-closure-audit-result.md`，包含 dependency DAG、crossing
ledger、remaining-blocker矩阵、repair DAG、临时目录清理与三个 clean 状态。不得修改代码、
merge、rebase、push、stable/live；完成后不得承接 repair。
