# P1-R01：删除 Package ABI 迁移别名

## 背景与目标

P1-A01 在候选 `7be9045` 上发现唯一阻断项：`artifact-identity` 仍公开导出三个注释明确要求
T06/T07 删除的旧概念名：`package_abi_hash`、`package_abi_identity` 和
`PACKAGE_ABI_IDENTITY_PREFIX`。这些入口目前只委托 canonical local ABI owner，没有第二套算法，但继续
保留会形成可扩散的双概念 API，也与 Phase 01 完成声明不一致。

本任务只关闭这一遗漏，不改变 package identity preimage、wire、prefix 内容或运行语义。

## 完成态

1. 删除三个旧定义及其 crate-root re-export；唯一公开概念名为 `package_local_abi_*` 与
   `PACKAGE_LOCAL_ABI_IDENTITY_PREFIX`。
2. runtime 测试辅助代码改用 `package_local_abi_identity`，不保留本地 wrapper 或新 alias。
3. single-source checker 对三个精确旧符号 fail-closed，并有负例 self-test；checker 自身的声明表/负例
   字符串是允许命中，不得借 allowlist 放过 production Rust 定义或调用。
4. 全仓 production/test Rust 调用点不再使用旧符号；Phase 结果记录首次 A01 FAIL、修复 commit 与重跑
   证据，并保持状态为等待 A01 复验。

## 写入边界

- `artifact-identity/src/package.rs`、`constants.rs`、`lib.rs`。
- 直接消费旧测试入口的 runtime host 测试代码。
- `scripts/check-artifact-identity-single-source.mjs` 及其直接 self-test 结构；若主 checker 已过长，新增规则
  优先放入现有职责相符的 helper，不把新的扫描算法塞进主文件。
- `phase-result.md` 的本次失败与修复证据。

不得修改 canonical identity 算法、DTO、compiler/runtime production load path、router 或 dev-sync。

## 验收

至少执行：

```bash
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only runtime
rg -n '\bpackage_abi_hash\s*\(|\bpackage_abi_identity\s*\(|\bPACKAGE_ABI_IDENTITY_PREFIX\b' \
  --glob '*.rs' --glob '!target/**' .
git diff --check
```

`rg` 最终必须没有 Rust 命中。对改动 Rust 文件运行 targeted rustfmt。若验证发现任何生产调用依赖旧 API，
先报告为新阻断项，不得恢复 alias。

## 独立复验

开发 Agent 提交后，由未参与本任务实现的只读 Agent 使用本文件复验删除范围、checker 负例有效性和上述
证据。复验 PASS 后才能合回 Phase 01 分支并重新执行 P1-A01。
