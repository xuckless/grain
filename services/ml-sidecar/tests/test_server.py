"""The HTTP contract against the stub backend (no model): `uv run --group dev pytest`."""

import io
import wave

import pytest
from fastapi.testclient import TestClient

from grain_sidecar.server import create_app
from grain_sidecar.stub_backend import StubBackend

TOKEN = "t" * 32
AUTH = {"Authorization": f"Bearer {TOKEN}"}


def silence(seconds=1.0, rate=44100):
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"\0\0" * int(seconds * rate))
    return out.getvalue()


@pytest.fixture
def client():
    return TestClient(create_app(StubBackend(), TOKEN))


def test_health_needs_the_token(client):
    assert client.get("/health").status_code == 401
    assert client.get("/health", headers={"Authorization": "Bearer nope"}).status_code == 401
    r = client.get("/health", headers=AUTH)
    assert r.status_code == 200
    assert r.json() == {"status": "ok", "backend": "stub", "device": "cpu"}


def test_generate_returns_wav_of_the_duration(client):
    q = {"task": "complete", "prompt": "warm piano", "seed": 7, "bpm": 96, "key": "C Major", "duration": 2}
    r = client.post("/generate", params=q, content=silence(), headers=AUTH)
    assert r.status_code == 200, r.text
    assert r.headers["content-type"] == "audio/wav"
    with wave.open(io.BytesIO(r.content)) as w:
        assert w.getnframes() == 2 * 44100
        assert w.getnchannels() == 2


@pytest.mark.parametrize(
    "query, detail",
    [
        ({"task": "nope"}, "task must be"),
        ({"task": "lego"}, "lego needs a track"),
        ({"task": "repaint", "start": 5, "end": 2}, "start < end"),
    ],
)
def test_generate_rejects_bad_requests(client, query, detail):
    q = {"seed": 1, "bpm": 120, "duration": 1, **query}
    r = client.post("/generate", params=q, content=silence(), headers=AUTH)
    assert r.status_code == 422
    assert detail in r.json()["detail"]


def test_generate_needs_a_wav_body(client):
    q = {"task": "complete", "seed": 1, "bpm": 120, "duration": 1}
    r = client.post("/generate", params=q, content=b"not audio", headers=AUTH)
    assert r.status_code == 422
