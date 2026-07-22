# P5-R27：Object Materialization Acceptance

未参与F25A/B实现的全新只读Agent。输入F25A/B exact clean candidate；不得编辑、修复、提交或运行real smoke/full/I16/
Host/stable。必验source facts是target/branch/field/default唯一owner；lowering不重复推断；normal-source accept/reject、nested
nominal Context/policy得到Object/Construct，Map/Json保持MapLiteral；missing/extra/ambiguous/targetless fail closed；无WS
名字特判或Runtime放宽。extra-review检查source/lowering没有新增第二type checker或巨型混合职责。

第一行只给`R27 PASS`或`R27 FAIL`。PASS只解锁I25。
