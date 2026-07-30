# P5-D58：Host Legacy Module Closure Audit

只读枚举runtime/host所有因F55B/D删除而失配的module/import/export/test：ArtifactGraph/cache/pointer selection、
program loader/linker/activation builder、RuntimeServiceConfig/ServiceRuntimeContext/legacy route。区分可整模块删除、
仍承载request heap/canonical assembly语义需迁移、以及真正canonical owner。

输出完整文件清单、删除顺序、单一写入owner、compile/正负探针与反搜；禁止编辑/gate。后续一次批量修复，不按
compiler首错逐项补丁，不恢复shim/alias。
