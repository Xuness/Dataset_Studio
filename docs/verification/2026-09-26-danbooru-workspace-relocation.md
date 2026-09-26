# Danbooru 生产者工作区归并到 F 盘

2026-09-26，按用户要求将 D:\Dataset\Danbooru 整体归并到 F:\Dataset\Danbooru。此前已完成三湖在线协议统一；本次统一生产者工作区的物理位置，不重新转换图片或在线库。

## 当前布局

三湖均使用 F:\Dataset\<站点名>，其中 ONLINE.json 指向 online 内的 v2 在线库，CURRENT.json 指向 indexes 内的 Store 生产者索引。Danbooru 的 archives、backups、daily、runtime、validation 一并迁入；Python 环境仍供三湖启动脚本共用。

E:\AI\AI_Dataset\<站点名> 继续存放图片和完整归档。Studio 中已登记的在线库路径无需再次修改。

## 验证及切换

- 复制期间持有 Danbooru 的日任务、归档 writer 和生产者索引锁，Studio 在线读取继续使用原 F 盘在线库。
- 1,215 个非运行时文件逐项 SHA-256 一致，共 244,524,282,598 字节。
- Python 3.11 环境在新位置重新生成启动器并重装固定依赖，24 个依赖版本一致，独立可执行入口和依赖检查通过。
- 更新 Store 配置、Danbooru/Yandere/Gelbooru 启动器、日任务和迁移脚本、验证脚本及当前操作文档。修复 requirements-lock.txt 漏列 APSW 的问题，避免 setup 同步依赖时将其卸载。
- 按用户最终确认，日任务自动化的工作目录路径同步改为 F 盘；运行时间、状态、模型、推理配置和项目保持原值。本地启动脚本亦使用新路径。
- 新位置的 status 与 daily-status 通过：11,413 个提交；11,593,632 个图像对象；13,418,451 条观察；两个日任务水位均为 12,260,651。
- 三湖 Studio API 的分页、详情、raw 与预览读取再次通过；38 项日任务清理及在线发布相关测试通过，PowerShell 启动器语法检查通过。

核验完成后删除 D 盘旧副本，共 4,916 个文件、244,717,907,593 字节。没有留下重定向或依赖旧 D 盘位置的虚拟环境入口。SQLite 的共享内存及空 WAL 属于可重建辅助文件，由运行时自行管理。

历史报告和 JSON 中的原路径作为当时证据保留。在线库保存 original_source_index 记录旧位置，当前 source_index 已改为 F 盘；代次、归档/在线/分析水位保持不变。

本机证据位于 .local/reports/danbooru-workspace-move-20260926/，日志位于 .local/logs/danbooru-workspace-move-20260926/，不随 Git 分发。
