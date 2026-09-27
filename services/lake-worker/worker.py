"""Engine-owned entry point; works with python -I and without an editable install."""

from pathlib import Path
import runpy
import sys

sys.stdin.reconfigure(encoding="utf-8")
sys.stdout.reconfigure(encoding="utf-8")
sys.stderr.reconfigure(encoding="utf-8")
sys.path.insert(0, str(Path(__file__).resolve().parent / "src"))
runpy.run_module("studio_lake.updates", run_name="__main__")
