# P5-F445C-R4 Official std authoring golden closure

状态：Ready。独立、test-only official std identity golden 审计与闭合。

## 直接父节点

- `P5-F445C-package-interface-identity-normalization-result.md`

F445G 的 File IR 版本升级会按设计改变 std package build，但其 focused driver test 在越过 build
断言后又暴露 `EXPECTED_PRELUDE_ID` 旧值。此 leaf 在 pre-I3 可编译点审计两者，给 F445G 提供
明确前后基线。

## 历史审计

分别使用两个隔离临时 worktree 与独立 Cargo target：

- F445C 前：`42edd1b5`
- F445C 后：`e48e7e11`

运行：

```bash
cargo test -p skiff-compiler \
  authoring::package_publication::tests::official_std_authoring_and_record_writer_are_fixed_and_deterministic \
  -- --exact --nocapture
```

记录 std package build actual。为到达后一断言，可在临时 worktree 暂时把
`EXPECTED_STD_BUILD_ID` 更新为观测 actual；再记录 prelude identity actual。若前后两个
production 点的 build 或 prelude actual 不一致，停止并上报。

R2 已独立证明 pre-I3 std package build candidate 为：

```text
skiff-package-build-v10:sha256:b3a2d0e8059cbad6f90c9e9dd48376e1d7c7a9c18de6063a60c2c24b8653a112
```

F445G 首次观测的 prelude candidate 为：

```text
skiff-prelude-v1:sha256:8ec6c2b3f4411b159d8b1b8dd2d55d036713a2533dd3aba043eb3d7fb020c76e
```

## 实现边界

仅当两个历史点产生相同 actual，才允许只更新：

`compiler/driver/authoring/package_publication/tests.rs`

中的：

- `EXPECTED_STD_BUILD_ID`
- `EXPECTED_PRELUDE_ID`

不得修改 production、identity 算法、File IR、timeout、其它 fixture 或 golden。正式更新后
focused test 必须完整通过；若出现第三个失败则停止记录，不扩大范围。

## 验证

```bash
cargo test -p skiff-compiler \
  authoring::package_publication::tests::official_std_authoring_and_record_writer_are_fixed_and_deterministic \
  -- --exact --nocapture
cargo fmt --check
git diff --check
```

## worktree 与提交

worktree：

`/Users/geek/workspace/skiff-p5-f445c-r4-std-authoring-goldens`

branch：

`codex/p5-f445c-r4-std-authoring-goldens`

base：`e48e7e11`，再 cherry-pick 本任务文档。

提交 test-only implementation，再只新增并提交：

`P5-F445C-R4-official-std-authoring-golden-closure-result.md`

最终 clean。不得派子 Agent、merge/rebase/push、stable/live/network。
