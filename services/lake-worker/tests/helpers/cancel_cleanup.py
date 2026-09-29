"""Abrupt-death cleanup fixture; only invoked with a pytest-owned controller root."""

import sys

from studio_lake.updates.cleanup import run
from studio_lake.updates.state import State

run(State(sys.argv[1]), sys.argv[2])
