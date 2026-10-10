"""One operation owns its download and TAR reservation through durable publication."""

from contextlib import ExitStack

from ..updates.sites import UpdateError
from ..updates.staging import StagingPlan, INITIAL_DOWNLOAD


class Spool(ExitStack):
    def __init__(self, lib, job_id, resources):
        super().__init__()
        self.lib, self.job_id, self.resources = lib, job_id, resources
        self.owned = set()

    def reserve(self, key, receipt_id):
        if key in self.owned:
            return
        directory = self.lib.cache / "pinterest_downloads" / self.job_id
        size = max((p.stat().st_size for p in (directory / (key + ".downloaded"),
                    directory / (key + ".partial")) if p.exists()), default=0)
        plan = StagingPlan(max(size, min(self.resources.max_download_bytes, INITIAL_DOWNLOAD)), None)
        lease = self.resources.try_reserve(directory, key, plan.peak(),
            related=[(self.lib.cache / "pack_staging", receipt_id)])
        if lease is None:
            raise UpdateError("UPDATE_SPACE", "Waiting for Pinterest download and publication space")
        self.callback(lease.release)
        self.resources.plan(directory, key, plan)
        self.owned.add(key)
