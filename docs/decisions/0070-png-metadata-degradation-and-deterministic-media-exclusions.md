# 0070 — PNG 元数据降级处理与确定性图片失败排除

日期：2026-10-09。状态：已实现。修订 [ADR 0051](0051-png-trailing-data-and-complete-icc.md) 的尾部数据预算与证据保存方式，以及 [ADR 0044](0044-lake-media-recovery.md) 中色彩错误的任务状态。

## 问题

2026-10-06 的 Danbooru 与 Gelbooru 补充新帖任务各有 3 条 `needs_review / image_decode_or_storage_error`，是同 MD5 的同一批文件。三份下载均与来源 MD5 一致，普通 Pillow 可完整解码，但被自有 PNG 检查拒绝：

- 1 份在 IEND 后附加 16.9 MB 不透明数据。尾部数据计入 16 MiB 元数据预算，因此超限。
- 2 份含两个 `Photoshop ICC profile` iCCP 块，解压后均为相同的 3,144 字节 sRGB。“重复 ICC”被直接拒绝。

这是继 ADR 0048、0051 之后的同类问题：白名单式检查遇到每种新的元数据异常都整张拒绝。另外，单图编码边界把 PNG 兼容错误、解码错误和磁盘错误合并为同一原因且不保留异常信息；MD5 已验证的原件每次重试都必然得到相同结果，却仍阻塞任务完成。

## 决定

PNG 兼容检查（兼容记录版本升为 3）：

- 按块类型分级。关键块、动画块，以及影响像素或颜色的辅助块（`tRNS gAMA cHRM sRGB iCCP sBIT cICP mDCv cLLi`）仍严格校验，CRC 错误或超预算时拒绝。其余辅助块视为不透明元数据。
- 不透明块 CRC 错误时，`tEXt zTXt iTXt eXIf` 沿用重算 CRC 后保留的做法；其他不透明块从解码副本移除（`opaque_chunk_crc_mismatch_removed_for_decode`）。不透明块会超出 16 MiB 元数据预算时，流式计算 SHA-256 并从解码副本移除（`oversized_opaque_chunk_removed_for_decode`），不读入内存，也不计入预算。
- 重复 iCCP 只保留第一个（与 libpng 一致）。后续块从解码副本移除（`duplicate_icc_removed_for_decode`），并记录解压后的配置 SHA-256、是否与保留配置一致；无法解压时记录错误文字。不再因重复配置拒绝。
- IEND 后的尾部数据不再计入元数据预算，只受下载单文件上限约束；仍计入 128 次恢复次数。
- 移除的字节不超过 64 KiB 时内联保存原字节，超过时只记录长度和 SHA-256：块为 `original_chunk_bytes` / `original_chunk_sha256`，尾部数据为 `bytes` / `sha256`。原件有来源 MD5 和地址，大块数据不放入资产详情 JSON。

单图结果：

- 原因码细分为 `image_source_incompatible`（PNG 兼容检查拒绝）、`image_decode_error`（Pillow 解码失败、解压炸弹等）、`image_color_profile_error` 和 `image_storage_error`（带 errno 的本地 I/O 错误，ENOSPC 仍按等待空间处理）。异常类型和信息截断为 200 字符，以 `detail` 写入媒体归档批次结果；任务条目表不变。
- 内容类失败（兼容、解码、色彩）在下载已通过来源 MD5 验证时记为 `unavailable`，任务以“完成 · 有未获取项”结束，原件留在任务暂存区。“重试未获取项”会把它们重置为待处理，使用当前规则对保留原件重新编码，不重新下载。
- 未通过 MD5 验证的内容失败可能来自传输损坏，因此仍记为 `needs_review`，并删除暂存原件，重试时重新下载。
- 配方排除（如拒绝透明图片的 `image_policy_rejected`）记为 `unavailable`。`image_storage_error` 与资源预算超限仍为 `needs_review`。

不变：像素、调色板、透明度和动画数据的完整性检查；Pillow 全局宽松解码参数；原图及保留动画策略返回完整下载字节；任务、资产与公开协议格式。历史条目的 `image_decode_or_storage_error` 保留原有界面说明。

## 验证

PNG、图片配方、媒体恢复和更新流水线专项共 180 项通过。新增用例覆盖：重复 iCCP（配置相同、不同、损坏）、不透明块 CRC 移除与超预算哈希、颜色块 CRC 仍拒绝、大尾部数据只存哈希且不占元数据预算、已验证失败的排除与本地重试、未验证失败的复审与原件删除、归档结果 `detail`。

在只读条件下，对 Danbooru 任务暂存区内的 3 份真实原件以原 WebP Q95/M4/2048 配方编码，均成功。兼容解码与普通 Pillow 解码逐像素比较 RGBA，并比较 ICC，均一致。16.9 MB 尾部数据的详情 JSON 为 1.2 KB。
