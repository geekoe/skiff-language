# P5-F302 Applied nominal compiler combined probe结果

状态：`FAIL`。不得解除`A2-language`或F269。

候选production checkpoint：`cadb128388e9b16e254c05cc8e7f8b2081a61b66`。

## 结果

| Selector | 枚举 | 运行 |
| --- | --- | --- |
| `file_ir_execution_type_representation` | 编译失败，exit 101 | 编译失败，0项执行 |
| `package_imports` | PASS，11项 | FAIL，2 passed / 9 failed |
| `test_artifact_identity` | PASS，5项 | FAIL，0 passed / 5 failed |

`git diff --check`与`git status --short`均clean。

## Finding wave

### F302-B1 旧测试API形状

`compiler/tests/file_ir_execution_type_representation.rs`仍导入已删除的
`BoundaryErrorContract`并给`BoundaryOperationContract`构造旧`errors`字段，导致selector在枚举前
失败。分类：mechanical integration fixture drift。

### F302-B2 std public schema误判

后两个selector共同先失败于：

```text
package skiff.run/std api websocket.<type> exposes generic declaration ...
generic declarations cannot be part of package public schema boundary
```

受影响的四个公开类型：

- `WebSocketConnectResult`
- `WebSocketConnection`
- `WebSocketIngressEvent`
- `WebSocketReceiveEvent`

这遮挡了`package_imports`中9项的目标语义断言和`test_artifact_identity`全部5项identity断言。
分类暂定为F301 package/public implementation；需要先区分“declaration自身有type parameters”和
“非泛型declaration内部引用builtin/container generic”并确认同类范围。

## 下一步

先执行`P5-F303-compiler-probe-failure-classification.md`有界只读归因；根据结果批量创建互不重叠的
修复节点。所有修复合流后必须重新运行同一combined probe，不能直接进入正式验收。

