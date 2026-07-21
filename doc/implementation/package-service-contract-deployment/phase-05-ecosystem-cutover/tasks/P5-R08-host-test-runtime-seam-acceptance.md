# P5-R08：Host Test-Runtime Seam Acceptance

未参与F08实现的独立只读Agent。输入为F08 exact clean integration commit/tree、D09/F08合同与聚焦证据；不得
编辑、提交、修复或给F04/R02 verdict。

必验：

- 25个编译错误全部关闭，Host production无legacy package-test import/dispatch/cache/synthetic execution；
- 没有从package/build/entrypoint字符串推导assembly triple、compat alias、pointer或第二admission/cache owner；
- capabilities/registration不再宣告package-test dispatch，normal request cancellation与assembly ingress未被误删；
- activation encode/decode严格使用现有direction-aware codec及相反正确方向，无transport schema变化；
- F03C剩余startup/lifecycle/request/drain/WS owner未提前实现，F04/test-runner/runtime-package-test无越界diff；
- `extra-review`确认删除909行混合职责seam后未把同一逻辑搬到其它大文件。

合流状态必须运行：

```bash
cargo check --locked -p skiff-runtime-host -p skiff-runtime-package-test -p skiff-test-runner
git diff --check
```

第一行只给`R08 PASS`或`R08 FAIL`。PASS只解锁F04原isolated Host probe与窄接收；FAIL给最小source反例与
唯一owner。
