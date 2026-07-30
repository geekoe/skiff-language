# P5-F55E：Host Linked Cache Residual

只改I55AB指出的host两处`LinkedProgramImageCache`残留，删除旧cache field/helper/test引用并保持request heap与
canonical assembly路径。不得恢复legacy cache。运行host聚焦/check、rustfmt/diff，提交单一commit。
