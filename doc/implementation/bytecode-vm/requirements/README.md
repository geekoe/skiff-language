# Bytecode VM Requirement Ledger

本目录承载 bytecode VM 实施范围的 requirement ledger 与 baseline benchmark manifest，是
[`../README.md`](../README.md)（实施范围冻结）和
[`../phases/phase-0-baseline-live.md`](../phases/phase-0-baseline-live.md)（Phase 0 交付物 1/5）的直接产物。

- [`ledger.md`](ledger.md)：三个 included commits（`46351f72`、`576bbd31`、`430e5ff2`）对
  `doc/architecture/` 与 `doc/reference/` 的每一个语义 hunk 的稳定 requirement 记录。
- [`benchmark-baseline.md`](benchmark-baseline.md)：Phase 0 冻结的 baseline benchmark manifest。

## 1. Schema

ledger 主体（[`ledger.md`](ledger.md)）按 commit 分三节，每节一张表，行字段固定为：

| 字段 | 含义 |
| --- | --- |
| `ID` | 稳定 requirement id，格式 `R-<编号>`，跨三个 commit 全局按 hunk 顺序连续编号（见 §2）。 |
| `Commit` | 该 hunk 来源 commit 的短 SHA（`46351f72` / `576bbd31` / `430e5ff2`）。 |
| `文件` | hunk 所在的 `doc/architecture/` 或 `doc/reference/` 路径（相对仓库根）。 |
| `语义要求` | 一句话精确描述该 hunk 要求的行为/合同。以 `430e5ff2` 的最终文本为权威表述（README §2.2）。 |
| `Owner` | 负责落地的代码 crate/模块路径（见 §4）；不确定时写 `TBD`。 |
| `状态` | `missing` / `existing-needs-proof` / `implemented` / `retirement-only`（见 §3）。 |
| `目标阶段` | 该需求主要在哪个实施阶段落地（`README.md` §5/§7 的 phase 编号，如 `Phase 1`、`Phase 3B`）。 |
| `证据` | 代码/测试/文档证据位置；本阶段全部为 `TBD（Phase 0 审计）`，除非已有明确实现证据（见 §3）。 |

约定：

- **语义 hunk**：一段删除/新增/修改的语义单元（一个文档小节、一组表达同一合同的段落），
  不是行级 diff。多个行级 `@@` hunk 属于同一语义单元的合并为一行；一个语义单元跨多个文件时
  按文件拆行。
- **deleted 页面**：`576bbd31` 删除的 9 个 `doc/architecture/` 页面与 1 个 `doc/reference/` 页面
  （README §3.2 中 `D` 状态）各占一行 `retirement-only` requirement：它们是被替换的概念面，
  其概念不得从被删文本重新引入，必须对照代码审计后删除/合并（README §3 注释）。
- **关键字迁移**（如 `const x` → `let x`）不单独成行；它归属所在文件的 value-semantics/
  writable-binding 语义行（`var`/`let`/`const` 三职责收敛由 `doc/reference/syntax.md` 与
  `static-semantics.md` 的对应行覆盖）。
- 后续设计 amendment 若进入 scope，必须按 [`../README.md`](../README.md) §2.2 的规则用其精确
  commit 追加记录，不能隐式并入。

## 2. ID 规则

- 格式：`R-<十进制编号>`，全局唯一、永不重用。
- 编号顺序 = included commits 顺序（`46351f72` → `576bbd31` → `430e5ff2`），每个 commit 内按
  文件在 `git show --stat` 中的出现顺序、文件内按文档小节顺序。
- 追加 amendment 时从当前最大编号继续，不重排既有编号。
- 引用的稳定写法：`R-012（46351f72，doc/architecture/bytecode-vm.md）`。跨行引用同一语义时
  只写一次 ID，其余行引用该 ID。

## 3. 状态枚举

| 状态 | 定义 | Phase 0 使用规则 |
| --- | --- | --- |
| `missing` | 目标行为尚未实现，需要新实现 + 聚焦测试。 | 默认状态；本阶段绝大多数行。 |
| `existing-needs-proof` | 代码中已有对应实现，但尚未有满足本 scope 的代码/测试证据（Phase 0 审计未完成）。 | 仅当 included 文档本身明确断言当前代码路径存在（并给出模块路径）时使用；证据列必须注明文档断言位置，最终证明仍由 Phase 0 审计给出。 |
| `implemented` | 已有实现且被代码/测试证据确认。 | 本阶段不标注（Phase 0 审计完成后翻状态）。 |
| `retirement-only` | 被删文档对应的 legacy 概念；要求"审计 + 删除/合并"，不需要新语义实现。 | 用于 10 个 deleted 页面及文档明示删除的机制（`RuntimeAssembly`、`call_suspend`、tree evaluator 等）。 |

**禁止**：把未确认的实现标为 `implemented`；`retirement-only` 不得被当成"文档删了就行"——
它要求对照代码的移除证据（README §8 完成定义第一条）。

## 4. Owner 映射规则

- Owner 是代码 crate/模块路径，取自 [`../README.md`](../README.md) §6 principal code areas；
  不是人的名字。
- 映射基准（§6.1–6.7）：
  - 6.1 artifact/identity：`artifact-model/src/...`、`artifact-identity/src/package_artifact/`
    （bytecode schema 倾向 `artifact-model/src/bytecode.rs`）；
  - 6.2 compiler：`compiler/source/src/...`、`compiler/lowering/src/...`、
    `compiler/emission/src/lib.rs`、`compiler/driver/pipeline/mod.rs`；
  - 6.3 linker/loader/image：`runtime/linked-program/src/...`、`runtime/linker/src/...`、
    `runtime/loader/src/...`；
  - 6.4 VM core/values/旧 evaluator：`runtime/model/src/...`、`runtime/eval/src/...`
    （窄 VM core 建议独立 `runtime/vm` crate，README §6.4 注释）；
  - 6.5 boundary/native/request/host：`runtime/boundary/src/...`、`runtime/native-contract/src/`、
    `runtime/native/src/...`、`runtime/capability-context/src/`、`runtime/request/src/...`、
    `runtime/host/src/...`；
  - 6.6 Actor：`runtime/eval/src/actor_*`、`runtime/host/src/host/actor_owner_execution.rs`、
    `router/src/actor/...`、`router/src/supervisor/actor*`、`router/src/task/...`；
  - 6.7 transport/router：`runtime/transport/src/...`、`router/src/dispatch/`、
    `router/src/session/`、`router/src/http/`。
- 涉及多个 §6 区域的语义行（如 `bytecode-vm.md` 的不变量总纲）写主要 owner，其余在语义要求
  文本中注明；不确定写 `TBD`，不允许把无 owner 的需求推到 Phase 9（phase-0 §6 停止条件）。
- 跨 repo 影响（如 `internals/agine` 的 chat smoke、`scripts/verify.mjs` live registry）在证据
  列注明，owner 仍是本仓库代码路径或 `TBD`。

## 5. 更新维护规则

1. **阶段完成翻状态**：每个 phase 的 focused tests + Live gate 通过并合并后，把该阶段
   `目标阶段` 命中且已实现的行的状态从 `missing` 翻为 `implemented`，并把测试/证据位置写入
   `证据` 列；`existing-needs-proof` 只有在证据满足 scope 要求时才翻为 `implemented`。
2. `retirement-only` 行在对应 legacy 代码被删除/合并并验证（`rg` 证据 + crate-DAG check）后
   翻为 `implemented`（删除即实现），并注明删除 commit/证据。
3. 每次状态/证据变化都必须与 phase 的 ledger 更新要求同步（README §7："Each phase should land
   with its own focused tests and a requirement-ledger update"）；不允许在结果出来后就地放宽
   （phase-9 §3.3）。
4. 新 amendment 追加新行（§2），不修改既有行的语义；发现既有行语义与最终文档不符时，开新行
   并在旧行标注 `superseded by R-<n>`。
5. Phase 0 审计发现"代码中已有实现"时，只翻 `existing-needs-proof` 并注明证据，不直接翻
   `implemented`；真正的 `implemented` 判定需要测试证据（README §2.2 的三种处置之一）。

## 6. 覆盖率核对（Phase 0）

用 `git show --format= --find-renames <sha> -- doc/architecture doc/reference` 提取三个 commit
的原始 diff，统计每个文件的 `@@` 行级 hunk 数；ledger 行数与之对照如下（语义 hunk 是对行级
hunk 的语义归并，行数允许少于 `@@` 数，但**每个文件至少一行、每个语义单元一行**）：

| Commit | 文件数 | 行级 `@@` hunk 数 | ledger 语义行数 | 覆盖 |
| --- | --- | --- | --- | --- |
| `46351f72` | 1（A） | 1（整文件 877 行新增） | 50（按 16 个小节归并） | 每节至少一行 |
| `576bbd31` | 40（31 M + 9 D） | 273 | 156 | 每文件至少一行；D 文件各一行 retirement |
| `430e5ff2` | 13（全 M） | 73 | 37 | 每文件至少一行 |

核对方法：对每个 commit 逐文件比对 `git show --format= --stat` 的文件清单与 ledger 行清单，
确认无文件漏记；对行级 hunk 密集的文件（`bytecode-vm.md`、`package-service-contract-deployment.md`、
`recoverable-value.md`、`runtime.md`、`static-semantics.md` 等）按小节逐段核对语义单元。
