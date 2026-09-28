"""One asynchronous media publication at a time, sharing the lake's metadata lane."""

from concurrent.futures import ThreadPoolExecutor
import threading


class Publications:
    def __init__(self):
        self.lock = threading.Lock()
        self.pool = ThreadPoolExecutor(max_workers=1, thread_name_prefix="lake-publish")
        self.future = None
        self.kind = None
        self.results = []

    def submit(self, kind, function, *args, results=None):
        if self.future is not None:
            return False
        self.kind, self.results = kind, results or []
        self.future = self.pool.submit(self._run, function, args)
        return True

    def _run(self, function, args):
        with self.lock:
            return function(*args)

    def collect(self, *, wait=False):
        if self.future is None or not wait and not self.future.done():
            return None
        future, kind, results = self.future, self.kind, self.results
        self.future, self.kind, self.results = None, None, []
        future.result()
        return kind, results

    def close(self):
        self.pool.shutdown(wait=True)
