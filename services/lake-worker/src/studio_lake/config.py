from dataclasses import dataclass
from pathlib import Path
import math
import tomllib


@dataclass(frozen=True)
class Config:
    root: Path
    cache: Path
    pack_target_bytes: int = 4 * 1024**3
    threads: int = 8
    memory_limit: str = "32GB"
    build_tags: bool = True
    build_tag_index: bool = False
    checkpoint_threshold: str = "512MB"
    page_size: int = 200
    storage_profile: str = "webp-2048-q95"
    timeout: int = 60
    request_delay: float = 0.5
    shard_target_bytes: int = 3_000_000_000
    row_group_rows: int = 128
    api_proxy: str = ""
    image_proxy: str = ""
    download_workers: int = 2
    encode_workers: int = 4
    image_requests_per_second: float = 2.75
    download_attempts: int = 3
    api_attempts: int = 4
    retry_base_seconds: float = 2
    retry_max_seconds: float = 60
    batch_flush_seconds: float = 600
    spool_limit_bytes: int = 8 * 1024**3
    max_download_bytes: int = 1024**3
    refresh_limit: int = 50000
    refresh_recent_days: int = 7
    refresh_older_days: int = 30
    work_retention_days: float = 7
    log_retention_days: float = 30
    orphan_retention_hours: float = 24

    def __post_init__(self):
        if not isinstance(self.build_tag_index, bool):
            raise ValueError("build_tag_index 必须是布尔值")
        for value in (self.work_retention_days, self.log_retention_days, self.orphan_retention_hours):
            if not math.isfinite(value) or value < 0:
                raise ValueError("清理保留时间必须为有限的非负数")
        root, cache = self.root.resolve(), self.cache.resolve()
        if root == cache or root in cache.parents or cache in root.parents:
            raise ValueError("主库和 SSD 工作目录必须互相独立，不能互相嵌套")
        if self.pack_target_bytes <= 0 or self.shard_target_bytes <= 0 or self.row_group_rows <= 0:
            raise ValueError("分片大小和行组大小必须为正数")
        if not 1 <= self.page_size <= 200:
            raise ValueError("API page_size 必须为 1–200")
        if self.storage_profile not in {"original", "webp-2048-q95"}:
            raise ValueError("未知图片保存策略")
        if not 1 <= self.download_workers <= 32 or not 1 <= self.encode_workers <= 32:
            raise ValueError("下载和编码 worker 数量必须为 1–32")
        if self.image_requests_per_second < 0:
            raise ValueError("图片请求限速不能为负数")
        if (
            self.download_attempts <= 0
            or self.api_attempts <= 0
            or self.retry_base_seconds < 0
            or self.retry_max_seconds < 0
        ):
            raise ValueError("无效重试设置")
        if self.batch_flush_seconds <= 0 or self.max_download_bytes <= 0 or self.spool_limit_bytes <= 0:
            raise ValueError("暂存和封存限制必须为正数")
        if self.spool_limit_bytes < 2 * self.max_download_bytes:
            raise ValueError("下载暂存额度至少需要容纳一份原文件与处理结果")
        if (
            self.refresh_limit < 0
            or self.refresh_recent_days <= 0
            or self.refresh_older_days < self.refresh_recent_days
        ):
            raise ValueError("无效元数据刷新设置")
        object.__setattr__(self, "root", root)
        object.__setattr__(self, "cache", cache)

    @classmethod
    def load(cls, path: Path):
        with path.open("rb") as f:
            obj = tomllib.load(f)
        s = obj["storage"]
        return cls(
            root=Path(s["root"]),
            cache=Path(s["cache"]),
            **{k: v for section in obj.values() for k, v in section.items() if k not in {"root", "cache"}},
        )
