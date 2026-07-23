# P5-F55B：Loader / Linker Legacy Chain Removal

独立worktree，独占runtime loader/linker旧graph/cache/pointer closure/link_runtime_program_image链及crate exports。
保留canonical runtime_assembly loader、assembly linker、AssemblyExecutionImage/SharedPackageLinkedImage。
删除PackageUnit/ServiceUnit/PublicationAbiUnit reader/validation/link plan consumers及旧测试，不留alias/shim/fallback。
暂不删linked-program facade或artifact-model/identity owner，留给后续F55C/T06。

运行loader/linker check、runtime_assembly/assembly正负测试、静态反搜、rustfmt/diff，提交单一commit。
不得改host/driver/linked-program/artifact model，禁止I02/R05/full gate。
