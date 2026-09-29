"""Hard-exit driver; accepts only paths supplied by the isolated pytest fixture."""
import sys
from studio_lake.updates.state import State
from studio_lake.updates import relocation

state = State(sys.argv[1])
if sys.argv[2] == "prepare":
    relocation.prepare(state, sys.argv[3])
else:
    relocation.apply(state, sys.argv[3], sys.argv[4], sys.argv[5])
    relocation.finish(state, sys.argv[3])
