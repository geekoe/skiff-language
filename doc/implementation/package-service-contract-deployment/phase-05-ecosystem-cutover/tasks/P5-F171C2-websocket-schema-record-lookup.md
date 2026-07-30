# P5-F171C2：WebSocket Schema Record Lookup

状态：Ready

## 直接父任务

- `P5-F171C-websocket-shared-schema-records-result.md`

## 当前断点

F171C把`websocket_ingress_context`硬改为只接受shared record map，使deployment projection现有的
value-owned publication map无法调用。两者都只需借用record，不应强迫生产投影改变所有权，也不应
复制payload。

## 范围

只修改artifact-model WebSocket schema helper及测试，并写result。

## 必须实现

- 定义最小只读record lookup抽象，或等价泛型接口，使
  `BTreeMap<Id, Record>`与`BTreeMap<Id, Arc<Record>>`均可零复制查询。
- helper内部只获得`&PackageSchemaTypeRecord`；closure、identity和cycle校验不变。
- 不提供复制转换函数，不恢复旧service-owned schema。
- 测试同时覆盖owned publication map和shared runtime map；shared路径Arc strong count不变。

## 验证

- artifact-model 相关测试和check；
- `cargo check -p skiff-deployment`；
- `git diff --check`；
- 独立提交并写result。
