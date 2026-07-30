# P5-F315 Representation wrap compiler producer result

状态：PASS。

实现提交：`287fec83a176b5c94fd34f8a6a7ca1bd1ba3ad02`。

## 结果

- validated representation target不再在lowering中丢失；每个显式constructor产生
  `ExprIr::RepresentationWrap`，child引用原payload表达式。
- plain、generic、nested与external owner target均保留exact type ref和ordered arguments。
- nested constructor产生nested wraps；没有隐式wrap，payload call只求值一次。
- direct throw继续使用required source site，throw value指向wrap，`payload_type`保持exact nominal。
- external refs收集与publication-local rewrite递归覆盖child和wrap target。
- compiler lowering goldens只迁移到File IR schema v8、format v6、identity prefix v8；没有恢复兼容路径。
- record等非representation constructor继续在source阶段拒绝。

## 验证

- lowering test list：52，非零。
- lowering full：52/52 PASS。
- `rustfmt --check`、`git diff --check`与提交空白检查：PASS。
- `_erased_wrapper_type`以及lowering中的旧v7/v5 generation反搜为零。
- 没有修改artifact/runtime/std/source/core/projection，也没有新增display/static fallback、
  named-union promotion或兼容分支。

