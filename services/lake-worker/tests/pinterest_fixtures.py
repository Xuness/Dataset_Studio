import hashlib
import io
import json
import uuid

from PIL import Image

from studio_lake.canonical import utc
from studio_lake.pinterest.http import Response
from studio_lake.pinterest.runner import Runner
from studio_lake.pinterest.service import Service
from studio_lake.updates.resources import Resources
from studio_lake.updates.state import State
from test_pinterest_manifest import pin


def png():
    output = io.BytesIO()
    Image.new("RGB", (8, 6), "red").save(output, format="PNG")
    return output.getvalue()


class MediaResponse:
    def __init__(self, data, etag=None):
        self.data, self.status_code = data, 200
        self.headers = {"Content-Length": str(len(data)), "ETag": etag or '"' + hashlib.md5(data).hexdigest() + '"'}

    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass

    def iter_content(self, size):
        for i in range(0, len(self.data), size):
            yield self.data[i:i + size]


class Fixture:
    def __init__(self, root, ids=("858146903966145189",), *, data=None, etag=None, pin_transform=None, budget=None):
        self.state = State(root / "controller")
        self.service = Service(self.state)
        self.lake = self.service.create_lake(dict(request_key=str(uuid.uuid4()), site="pinterest",
            media_root=str(root / "media"), index_root=str(root / "index")))
        self.spec = dict(version=1, collector="pinterest_web_v1", library_id=self.lake["library_id"],
                         seeds=[dict(kind="pin", id=v) for v in ids], run_budget=budget or {})
        self.job = self.service.create(dict(request_key=str(uuid.uuid4()), definition=self.spec))
        self.pin_calls, self.media_calls = [], []
        self.data, self.etag, self.pin_transform = data if data is not None else png(), etag, pin_transform

    def factory(self, root, context, *, cancelled):
        fixture = self

        class Pins:
            def pin(self, identity):
                fixture.pin_calls.append(identity)
                value = pin(identity)
                if fixture.pin_transform:
                    fixture.pin_transform(value)
                body = json.dumps(dict(resource_response=dict(status="success", data=value))).encode()
                return Response(body, "/resource/PinResource/get/", dict(options=dict(id=identity, field_set_key="detailed")),
                                200, utc(), context)

            def close(self):
                pass
        return Pins()

    def get(self, url, **kwargs):
        self.media_calls.append((url, kwargs))
        return MediaResponse(self.data, self.etag)

    def close(self):
        pass

    def runner(self):
        return Runner(self.state, resources=Resources(reserve_bytes=0), client_factory=self.factory, image_http=self)

    def run(self):
        return self.runner().run(self.job["id"], time_slice=60)
