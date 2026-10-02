# 0051 — PNG 尾部附加数据与完整 ICC 的兼容解码

日期：2026-10-02。状态：已实现，扩展 [ADR 0048](0048-bounded-png-metadata-recovery.md)。

## 问题

Danbooru 补充新帖任务有 27 张 PNG 在重复重试后仍被标记为 `image_decode_or_storage_error`。27 份下载均匹配来源 MD5，Pillow 12.2.0 均可完整读取像素。23 份在合法 IEND 后含附加数据，4 份 iCCP 的 zlib 流未正常结束，但已解出完整的 3,144 字节 ICC。此前自有 PNG 检查在这两处直接拒绝，因此重下相同原件无法恢复。

## 决定

- PNG 流检查到首个长度为零、CRC 正确的 IEND 为止。其后字节作为不透明附加数据处理，计入原有 16 MiB 元数据预算；在解码副本中移除，完整原字节、偏移、长度与 SHA-256 写入来源处理详情。它们不作为额外图片或动画帧读取。
- ICC 解压继续采用 4 MiB 上限。仅在外层 iCCP CRC 正确、zlib 未报告压缩错误且没有多余数据、解出字节数等于 ICC 头部声明大小、`acsp` 签名正确、非空标签表和全部标签范围完整且 LittleCMS 能打开时，允许使用未正常结束压缩流中已解出的完整配置。
- 这类 ICC 从解码副本中移除压缩块，再把相同的解压后配置字节放回该图片的 `info`。原始完整 iCCP 块与恢复动作均记录；不伪造原流通过结束校验的证据，也不声称通过了完整 ICC 标准符合性验证。
- PNG 主体截断、缺少 IEND、非空 IEND、关键块/像素/透明度/动画 CRC 错误继续拒绝。ICC 数据不足、尺寸或标签越界、压缩错误、压缩尾部多余数据，以及“未结束流同时伴随 iCCP CRC 错误”继续拒绝。
- 兼容记录版本升为 2，新增 `trailing_data_retained_outside_decode` 与 `incomplete_icc_stream_complete_profile_retained` 动作。恢复次数仍最多 128，尾部数据也计入次数与内存预算；Pillow 的全局宽松解码和文本解压参数保持原值。
- 原图及保留动画策略仍返回完整下载字节。转码沿用用户配方，来源帖子原始元数据和 Delete/Ban 标记不变；任务、资产和公开协议不变。

IEND 的边界依据 [W3C PNG 第三版 11.2.4](https://www.w3.org/TR/png-3/#11IEND)。ICC 检查参考 [ICC 的配置评估说明](https://www.color.org/profiles/assessment/)，这里只实施上述有界兼容检查。zlib 完整流和 Adler-32 的区别见 [RFC 1950](https://www.rfc-editor.org/rfc/rfc1950#section-2.2)。

## 验证

专项覆盖原字节及透明通道保留、完整/不完整 ICC、CRC、压缩错误、结构损坏、累积预算、原图/动画策略与生产归档路径。27 张正式缓存图片在隔离湖全部完成 WebP Q95/M4/2048 编码与归档，恢复后的 RGBA 像素和 ICC 字节逐一与直接解码结果一致。正式重试和完整检查结果见 [验收记录](../verification/2026-10-02-danbooru-png-recovery.md)。
