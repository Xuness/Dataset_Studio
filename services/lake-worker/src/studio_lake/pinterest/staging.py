"""One operation owns its download and TAR reservation through durable publication."""

from contextlib import ExitStack

from ..updates.sites import UpdateError
from ..updates.staging import StagingPlan, INITIAL_DOWNLOAD


class Spool(ExitStack):
    def __init__(self, lib, job_id, resources):
        super().__init__()
        self.lib, self.job_id, self.resources = lib, job_id, resources
        self.owned = set()
        self.leases = []

    def attach(self, lease):
        self.leases.append(lease)
        self.callback(lambda: lease.release() if self.resources.retained.get(lease.key) is not lease else None)

    def retain(self):
        for lease in self.leases:
            self.resources.retain(lease)

    def metadata(self, receipt_id, needed):
        directory = self.lib.cache / "pinterest_receipts" / self.job_id
        lease = self.resources.retained_lease(directory, receipt_id)
        if lease is None:
            lease = next((v for v in self.leases if v.key == (directory, receipt_id)), None)
        if lease is None:
            lease = self.resources.try_reserve(directory, receipt_id, needed)
            if lease is None:
                raise UpdateError("UPDATE_SPACE", "Waiting for Pinterest receipt space")
            self.attach(lease)
        else:
            self.resources.stage_size(*lease.key, needed)
        return lease

    def reserve(self, key, receipt_id):
        if key in self.owned:
            return
        directory = self.lib.cache / "pinterest_downloads" / self.job_id
        old = self.resources.retained_lease(directory, key)
        if old is not None:
            self.owned.add(key)
            return
        size = max((p.stat().st_size for p in (directory / (key + ".downloaded"),
                    directory / (key + ".partial")) if p.exists()), default=0)
        plan = StagingPlan(max(size, min(self.resources.max_download_bytes, INITIAL_DOWNLOAD)), None)
        lease = self.resources.try_reserve(directory, key, plan.peak(),
            related=[(self.lib.cache / "pack_staging", receipt_id)])
        if lease is None:
            raise UpdateError("UPDATE_SPACE", "Waiting for Pinterest download and publication space")
        self.attach(lease)
        self.resources.plan(directory, key, plan)
        self.owned.add(key)
