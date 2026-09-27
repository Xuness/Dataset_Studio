"""Isolated process for hard-kill recovery tests; all HTTP goes to the fixture server."""

from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))

import requests

from studio_lake.updates.runner import Runner
from studio_lake.updates.state import State
from test_updates import FakeSite, post


class LocalImages:
    def __init__(self, endpoint):
        self.endpoint = endpoint
        self.session = requests.Session()
        self.session.trust_env = False

    def get(self, _url, **kw):
        return self.session.get(self.endpoint, **kw)


if __name__ == "__main__":
    root, endpoint = Path(sys.argv[1]), sys.argv[2]
    data = (root / "original.png").read_bytes()
    runner = Runner(
        State(root / "control"),
        {"yandere": FakeSite("yandere", [post("yandere", 11, data)])},
        image_http=LocalImages(endpoint),
    )
    runner.serve()
