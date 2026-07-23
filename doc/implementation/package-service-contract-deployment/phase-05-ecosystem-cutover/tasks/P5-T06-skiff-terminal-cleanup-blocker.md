# P5-T06：Terminal Cleanup Blocker

结论：TASK_NOT_EXECUTABLE at R02 checkpoint `8ecf41ce9581714b8c72617d4d0c612982dc6899`。

旧PackageUnit/ServiceUnit/PublicationAbiUnit仍被33个T06范围外production文件直接消费，覆盖runtime
loader/linker/host/driver及linked-program public aliases。直接删除会破坏编译；迁移consumer超出原合同，且禁止shim。
拆D55审计并新增terminal consumer checkpoint后再恢复T06。
