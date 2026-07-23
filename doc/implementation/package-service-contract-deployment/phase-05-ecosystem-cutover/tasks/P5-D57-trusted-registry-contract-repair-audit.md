# P5-D57：Trusted Registry Contract Repair Audit

F56C2/C3/C4 TASK_NOT_EXECUTABLE后，三个全新只读分片并行：

- D57A：将外部activation surface收敛为单一`activation.activate`，区分coordinator内部backend transaction。
- D57B：定义path-free public pointer DTO与filesystem storage pointer转换owner；冻结source→IR→projection的
  trusted capability requirement语义。
- D57C：冻结StoreApi crate位置、Host store source/injection、registry principal allowlist/config owner及typed
  native dispatch ABI。

输出可直接实现的共享checkpoint、互斥owner和负例；禁止编辑/gate。不得dual path、ambient principal/path或
普通package自动获权。
