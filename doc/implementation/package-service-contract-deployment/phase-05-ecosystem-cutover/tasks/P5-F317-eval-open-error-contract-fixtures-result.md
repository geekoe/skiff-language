# P5-F317 Eval open error contract fixture batch result

状态：PASS。

实现提交：`9c3a948cd62fd5f26e9336c0993a3c7ce8dc4eb1`。

## 结果

- 三份授权eval fixture中的`BoundaryErrorContract` import、构造和`errors`字段已经删除。
- source-inline effect fixture仍保留`Failure` Package requirement、typed throw/package symbol、
  exact catch与随后response的完整行为。
- helper与断言改为描述开放service error channel，不再暗示operation发布closed throw set。
- `runtime/eval`中`BoundaryErrorContract|errors: BoundaryErrorContract`以及旧typed-contract措辞反搜为零。
- 没有修改production、representation、request、host或service wire。

## 验证

- `git diff --check`与三份文件的`rustfmt --check`：PASS。
- agent分支运行eval selector时，被当时尚未合入的`ExprIr::RepresentationWrap` linker非穷尽错误遮挡；
  遮挡点是`runtime/linker/src/linker/file_conversion.rs`。F316随后已合入linked consumer，最终编译验证由
  representation combined probe统一负责。

