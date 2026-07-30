# P5-F287 Std error surface migration result

状态：`PASS`；语言与 std 不再存在 `ErrorPayload` marker，固定公开内部错误为
`std.service.InternalError`。

## Exact candidate

- implementation commit：
  `7143b49e82c0756060f378dd6122132beacb75e9`
- integration merge commit：
  `527e36bf`
- 直接父：
  `P5-F287-std-error-surface-migration.md`

## 已冻结事实

1. 任意允许抛出的自定义名义类型不再需要实现 `ErrorPayload`。
2. compiler builtin、source 注入、prelude、std 声明和 tooling 中的 marker 已删除。
3. 原有 std 错误仍是各自的普通名义类型；`std.resource.ResourceError` 没有被提升为
   platform fixed error。
4. 新增普通 public、`PublicNameable + SchemaClosed` record：

   ```skiff
   std.service.InternalError {
     message: string,
     traceId: string,
     errorId: string,
   }
   ```

5. 它不是 native、prelude alias，也不等同于 generic runtime diagnostic 的字符串
   `"InternalError"` code。
6. production 与授权 fixture 反向搜索中，`implements ErrorPayload`、旧
   `std.error.InternalError` 和 bare/native `InternalError` 均为零。

## 验证

实现分支实际通过：

```text
compiler-core prelude_registry                 6/6
compiler-source semantic::interface            6/6
compiler-source prelude_registry              21/21
compiler package_std_schema                    8/8
compiler authoring::package_publication        5/5
nominal-interface focused probe                1/1
ignored F18 combined identity probe            1/1
scripts/check-skiff-source-layout.mjs          PASS
vscode/scripts/test-grammar.mjs                PASS
git diff --check                               PASS
```

当前 projection 生成的 identity 为后续 consumer 的输入事实，不是 runtime hard-coded alias：

- std schema index：
  `skiff-package-schema-index-v1:sha256:91fde4e37e4d120cc776a0486db38fd6bf538e69575704e042d3d7ec2463b997`
- `std.service.InternalError` schema type：
  `skiff-package-schema-type-v1:sha256:91c13b0e43f83941edd080b664ec3a35b1d2fbcf6e0be14d4380863b6a972978`

F288 切换 artifact identity generation 后，combined owner 必须按最终产物机械刷新授权 golden；
不得重新引入 marker、旧路径或 compatibility reader。
