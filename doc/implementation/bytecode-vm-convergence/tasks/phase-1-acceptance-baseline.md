# Phase 1 Acceptance rollout baseline（R0）

> Status: recorded at `2be7d126`
>
> 用途：O1/T-R/Gate/Acceptance 链（R1..R5）每一步引用本文件区分**新红**与**旧红**。
> 旧红 = 本文件列出的已知预存失败；新红 = 本文件以外的任何失败，由对应 step owner 修复或补充 waiver。
>
> 记录时间：2026-08-13（Asia/Shanghai），integrator 执行。

## 1. 验收基线 commit/tree

| 项目 | 值 |
| --- | --- |
| integration 分支 | `codex/bcvm-p1-integration` |
| worktree | `/Users/geek/workspace/skiff-bcvm-p1-integration` |
| baseline commit | `2be7d126cbcbc670d8e6fd9b0f770f9e8cbee85b` |
| baseline tree | `92c6920b65005a720e69edc4e9c098f6f1508f3e` |
| worktree 状态 | clean（`git status --porcelain=v1 --untracked-files=all` 为空，本文件提交前） |
| main checkout | `/Users/geek/workspace/skiff` 在 `main` @ `b2bfdb0f` |

## 2. 绿色基线：三包测试

命令（由 MAP1 Revision 11 的 L5 owner 记录，integrator 未重跑，只核对持久证据哈希）：

```bash
cargo test -p skiff-runtime-scheduler -p skiff-runtime-request -p skiff-runtime-host
```

| 项目 | 值 |
| --- | --- |
| 退出码 | 0 |
| scheduler | 36 lib + 9 integration，0 failed/ignored/skipped |
| request | 46 lib + 14 integration，0 failed/ignored/skipped |
| host | 176 lib + 1 integration，0 failed/ignored/skipped |
| doc-tests | 9（scheduler 1 + request 7 + host 1），0 failed/ignored/skipped |
| 日志 | `/tmp/skiff-p1-l5-correction-full.log` |
| 日志 SHA-256 | `851d52fa168f6b21f58bb31d90256e17d798c1b4af09fc393f844355c392c476`（integrator 于 2026-08-13 复核一致） |

该绿色基线是 O1..R5 全链的“必须保持绿”集合。任何后续步骤导致三包测试退出码非 0 或出现
failed/ignored/skipped，都属于新红，不因“本来就有红”而豁免。

## 3. 已知预存红清单（旧红）

### R-FMT：rustfmt 基线漂移

| 项目 | 值 |
| --- | --- |
| 命令 | `cargo fmt --check --all` |
| rustfmt 版本 | `1.8.0-stable (6b00bc3880 2025-06-23)` |
| 退出码 | 1 |
| 现象 | 工作区 652 个 `Diff in` 条目，包含未改动 crate（历史漂移，非本链引入） |
| 日志 | `/tmp/skiff-p1-baseline-fmt.log`（integrator 于 2026-08-13 复跑） |
| 日志 SHA-256 | `58d73a8428d5497ee9cc61afb9fc28e1ebb416bde8cc590ff7ec727d9aad568d` |

判定规则：本链不要求修复全工作区漂移。后续步骤新增/修改的文件不得使**其自身**偏离 rustfmt
规范（对新写入代码按仓库规范格式化）；`cargo fmt --check --all` 的总数只能因本链改动减少或
保持不变，不得因本链改动新增未格式化文件。任何在本链写入文件内的新 fmt 告警 = 新红。

### R-CLIPPY：admission.rs 预存 deny

| 项目 | 值 |
| --- | --- |
| 命令 | `CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target cargo clippy -p skiff-compiler-emission --all-targets` |
| 退出码 | 101 |
| 现象 | `clippy::never_loop`（deny by default）@ `compiler/emission/src/bytecode/admission.rs:60`（`for declaration in &unit.type_table {`），另含 4 个 advisory warning（如 `manual_contains`） |
| 是否被 L5 链改动 | 否：`git diff 296462db^..6d0d215b -- compiler/emission/src/bytecode/admission.rs` 为空；该文件上次变更在 `029bde09` |
| 日志 | `/tmp/skiff-p1-baseline-clippy.log`（integrator 于 2026-08-13 复跑） |
| 日志 SHA-256 | `2a0dd4353cf3b96ef2f435244e661edbafa9a5a73988ad30439d4d7d925d0e44` |

判定规则：`admission.rs:60` 的 `never_loop` 与同文件 advisory warning 为旧红。其他 crate/文件
的 clippy error 或 warning 均为新红。

## 4. 后续步骤引用方式

每步完成时按下列格式报告测试状态，并明确区分：

```text
green baseline: cargo test -p skiff-runtime-scheduler -p skiff-runtime-request -p skiff-runtime-host -> exit 0
old red (waived): R-FMT 652 漂移（未新增本链文件）、R-CLIPPY admission.rs:60 never_loop
new red: <无 / 清单>
```

任何 owner 遇到本清单外的失败必须先判定是“本步引入的新红”还是“需要补充 basline waiver 的既有红”，
并把结论写进 handoff；不允许在未记录的情况下接受带新红的里程碑。
