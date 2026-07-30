# P5-F436A AIHub interface explicit self receivers

状态：Ready。低风险 Internals source repair。

## 直接父节点

- `P5-F435A-generic-call-concrete-return-typing-result.md`
- `P5-F434A-aihub-correlation-free-http-combined-result.md`

F435A 已证明 shared compiler blocker 修复有效，并冻结下一处真实 AIHub canonical publish 首错；
引用链继续到唯一权威设计。

## 精确输入与 DAG

| repo | commit | tree |
| --- | --- | --- |
| Skiff integration | `6276ddbea46184ccc4251aa3173ab411f38ac28a` | dispatch 时记录 |
| Internals integration | `58950858a2e2cbf2bd95443d5e0704d0d29e7706` | `db88355a103e6e1939e9969756501c7f656c1344` |

本节点只修复 AIHub source declaration，使 canonical publish 越过 object-safety 诊断；完成后解除
AIHub correlation-free HTTP combined 的下一次执行。当前仍是实现检查点，不是稳定候选。

## 已确认 owner

当前 AIHub service 只有两个 interface declaration：

- `aihub/service/internal/aihub_service.skiff::AihubManagedLlmClient`
- `aihub/service/internal/provider_catalog.skiff::AihubProviderCatalog`

两者的 `impl` method 已显式声明 concrete `self`，但 interface method 都缺少第一参数
`self: Self`。仓库已有 `packages/llm-api`、Codex Relay 等 canonical 示例。

## 写入范围

只允许：

- `aihub/service/internal/aihub_service.skiff`
- `aihub/service/internal/provider_catalog.skiff`
- 与这两个 declaration 直接对应且确有必要的 AIHub source test
- 本 leaf result

禁止修改 compiler、runtime、router、test-runner、AIHub HTTP payload/stream、provider transport、
service/API authoring、Agine、Codex Relay、skiff-packages或其它 Internals owner。若正确修复需要
改变 interface/object-safety 语言语义或出现新的独立 owner，记录精确首错并停止。

## 必须实现

1. `AihubManagedLlmClient` 的三个 method 与 `AihubProviderCatalog` 的两个 method 都以
   `self: Self` 为第一参数。
2. 对应 concrete `impl` signature、返回类型和业务实现不变。
3. 反向检查 AIHub service 不再存在缺少 `self: Self` 的 interface method。
4. canonical AIHub `type-check` 必须越过 F435A 记录的 object-safety 错误；若随后出现独立
   blocker，只记录 failure classification，不顺手扩大。

## 验证

本 Agent 是以下聚焦证据的唯一 owner：

```bash
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run type-check
git diff --check
```

只有 `type-check` 通过时，才可运行不重复 publish 的最小 AIHub source/interface 聚焦测试；不得运行
完整 combined、live provider、stable、build/dev/start 或固定端口 workload。

## Worktree 与交付

- Internals worktree：`/Users/geek/workspace/internals-p5-f436a-aihub-interface-receivers`
- Skiff worktree：`/Users/geek/workspace/skiff-p5-f436a-aihub-interface-receivers`
- 分支：`codex/p5-f436a-aihub-interface-receivers`

启动后 5 分钟内完成第一次实际 source 修改，或报告具体未知量。提交 Internals implementation；
随后在 Skiff worktree 新增并提交
`P5-F436A-aihub-interface-self-receivers-result.md`。返回两个 commit/tree、聚焦证据与 clean
状态。不得 merge、rebase、push、访问 stable/live 或承接 combined。
