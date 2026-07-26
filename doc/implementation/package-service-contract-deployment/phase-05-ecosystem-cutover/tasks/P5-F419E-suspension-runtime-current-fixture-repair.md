# P5-F419E Suspension runtime current fixture repair

状态：Ready。

## 直接父节点

- `P5-F419C-suspension-runtime-combined-fixture-repair-result.md`

F419C已经完成canonical callable id与合法空API控制文件适配，并精确暴露三个后续fixture authoring缺口。
本节点只完成这三个缺口，不修改production或validator。

## 精确起点与独占范围

- integrated F419C checkpoint：
  `b7f7530d4b28b5c84e849a0ea2358c02ed435193`；
- F419C test adaptation：
  `332c98d588311f0b260ff3213f8b5488f103c193`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明三个commit均为HEAD ancestor。

唯一允许写入：

```text
runtime/eval/src/assembly_execution/ordinary/tests.rs
runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs
本任务 result
```

禁止修改production、shared test support、validator、artifact model、compiler、deployment、tooling或设计；
不得派子 Agent、merge/rebase/push、stable/live。

## 精确修复

1. package-direct六个fixture：
   - callee public path `mutate` 使用F419C canonical callable id；
   - 为unary与stream callee在 `implementation_links.functions` 补同一个FileIR identity、module、
     executable index、symbol与exact signature的canonical function export；
   - public symbol、semantic facts、boundary projection、callable link、caller external ref与package
     direct target逐值一致；
   - 不放宽canonical id、public-function target或link validation。
2. typed-throw source fixture：
   - `echo(string) -> string` operation的 `package_type_requirements` 必须是exact operation-reachable
     empty closure；
   - consumer对`errors.Failure`的direct package dependency、test double typed throw、
     `catch<errors.Failure>`和ordered second response保持。
3. std source fixture：
   - `execute_overlay_case` / `hydrate_packages` 使用已有 `HydratedPackageCode` current路径；
   - overlay携带compiler-emitted schema index/records；
   - canonical dependencies从 `CanonicalArtifactStore` 解析真实schema index/records；
   - 不绕过或放宽 `test_support` 对public schema hydrate的fail-closed检查。

不得改变8个测试的same-heap、effect consumption/finalization、request subset、stream order、
diagnostic、typed catch或response语义。

## 验证与交付

先逐条执行F419C result列出的8个exact tests，然后使用共享target先listing再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval assembly_execution -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval assembly_execution
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval --lib -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-eval --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-runtime-eval
cargo fmt --all -- --check
git diff --check
```

预期8条exact全绿、focused `92/92`、full eval `216/216`。写
`P5-F419E-suspension-runtime-current-fixture-repair-result.md`，记录exact commit/tree、三个authoring
数据流、8项闭合、实际计数与边界。提交并保持clean；不merge/rebase/push。若还暴露未授权的新owner或
production修改需求，返回`TASK_SCOPE_EXPANDED`。
