"""Install only the pinned public DINOv3 inference package into this venv."""
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import sys
import sysconfig
import urllib.request
import zipfile

REVISION = "8315d80ad58347ea119ffe7e415af2e6a42a6463"
URL = f"https://codeload.github.com/spawner1145/comfyui-kaloscope/zip/{REVISION}"
destination = Path(sysconfig.get_path("purelib"))
if Path(sys.prefix) == Path(sys.base_prefix):
    raise SystemExit("Use the isolated visual-worker environment")
marker = destination / "studio_kaloscope_architecture.json"
if marker.exists() and json.loads(marker.read_text())["revision"] == REVISION:
    print("Kaloscope architecture already installed:", REVISION)
    raise SystemExit(0)
with urllib.request.urlopen(URL, timeout=120) as response:
    archive = response.read(32 << 20)
with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
    selected = []
    for info in bundle.infolist():
        parts = PurePosixPath(info.filename).parts
        if len(parts) < 3 or parts[1] != "kaloscope_dinov3" or info.is_dir():
            continue
        if any(p in ("..", ".") for p in parts):
            raise ValueError("unsafe architecture archive")
        relative = Path(*parts[1:])
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(bundle.read(info))
        selected.append(relative.as_posix())
    if "kaloscope_dinov3/architecture.py" not in selected or "kaloscope_dinov3/LICENSE.md" not in selected:
        raise ValueError("incomplete DINOv3 architecture archive")
marker.write_text(json.dumps({"revision": REVISION, "url": URL,
                             "archive_sha256": hashlib.sha256(archive).hexdigest(), "files": selected}, indent=2))
print("Installed pinned Kaloscope architecture:", REVISION)
