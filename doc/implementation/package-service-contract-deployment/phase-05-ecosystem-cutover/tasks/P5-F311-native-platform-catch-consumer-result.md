# P5-F311 Native platform catch consumer结果

状态：Completed。

任务提交：`2ee87a687da584497d68ebfa36ce2b9349ab8e01`。

集成提交：`c0c922dff9c726f4fd421c54ce9ae1bb3379d278`。

## 结果

- dynamic DecodeTarget仅通过`decode_target_error_code`与finite
  `PlatformBuiltinErrorIdentity::from_symbol`组合，unknown为`None`；
- BytesDecode、DbDecode、File、Http、Cancel、Timeout使用exact enum identity；
- ResourceError与ordinary diagnostics为`None`；
- Opaque继续转发inner projection；
- 删除旧`TypeIdentity`与fake `test.*` platform identity。

## 验证

- native list/full：PASS，96/96；
- crate fmt与`git diff --check`：PASS；
- dynamic production路径只剩唯一canonical registry lookup。

R3关闭。

