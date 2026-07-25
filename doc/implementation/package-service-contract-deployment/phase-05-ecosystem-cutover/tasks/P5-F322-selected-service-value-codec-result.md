# P5-F322 Selected service value codec result

状态：PASS。

实现提交：`bdfb8890acd4b1c1ae2482d14d59b1eed6926cea`。

## 结果

- 新增公开`ServiceValueSelection::{Root, NamedUnionBranch(index)}`与带selection的decode结果。
- record/representation只接受`Root`；named union编码必须显式选择branch，不能shape-first-match。
- named union解码保留root binary ordinal；same-shape branch 0/1 round-trip后仍可精确区分。
- wrong root/branch组合、越界ordinal、payload不匹配、tamper与trailing bytes严格拒绝并回滚heap。
- nested union继续递归校验，只把root selection返回给caller。
- `ServiceResponse`及其它现有boundary限制保持；ordinary encode/decode API回归不变。
- binary generation、legacy/default/dual format均未改变。

## 验证

- runtime-boundary test list：181，非零。
- runtime-boundary full：181/181 PASS。
- crate `rustfmt --check`与`git diff --check`：PASS。
- 修改仅限`service_value_plan.rs`、其测试及`lib.rs`导出。

