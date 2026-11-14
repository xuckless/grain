"""The HTTP contract the app speaks (desktop: engine-native `generate.rs`; web: a configured
remote backend). Every request carries `Authorization: Bearer <token>`.

GET  /health    → {"status": "ok", "backend": str, "device": str}
POST /generate  query: task (complete|lego|repaint), prompt, seed, bpm, key, duration,
                [track] (lego), [start, end] (repaint, seconds)
                body: the conditioning audio as a WAV file
                → 200 audio/wav (the generated audio), or 4xx/5xx {"detail": str}
"""

import hmac

from fastapi import Depends, FastAPI, Header, HTTPException, Query, Request, Response

TASKS = ("complete", "lego", "repaint")


def create_app(backend, token: str) -> FastAPI:
    app = FastAPI(title="Grain sidecar", docs_url=None, redoc_url=None, openapi_url=None)

    def authorized(authorization: str = Header(default="")):
        expected = f"Bearer {token}"
        if not hmac.compare_digest(authorization.encode(), expected.encode()):
            raise HTTPException(status_code=401, detail="missing or wrong token")

    @app.get("/health", dependencies=[Depends(authorized)])
    def health():
        return {"status": "ok", "backend": backend.name, "device": backend.device()}

    @app.post("/generate", dependencies=[Depends(authorized)])
    async def generate(
        request: Request,
        task: str = Query(...),
        prompt: str = Query(""),
        seed: int = Query(..., ge=0),
        bpm: float = Query(..., gt=0),
        key: str = Query(""),
        duration: float = Query(..., gt=0, le=600),
        track: str = Query(""),
        start: float = Query(0.0, ge=0),
        end: float = Query(-1.0),
    ):
        if task not in TASKS:
            raise HTTPException(status_code=422, detail=f"task must be one of {TASKS}")
        if task == "lego" and not track:
            raise HTTPException(status_code=422, detail="lego needs a track")
        if task == "repaint" and not (end > start):
            raise HTTPException(status_code=422, detail="repaint needs start < end")
        audio = await request.body()
        if not audio.startswith(b"RIFF"):
            raise HTTPException(status_code=422, detail="the body must be a WAV file")
        try:
            wav = backend.generate(
                task=task, prompt=prompt, seed=seed, bpm=bpm, key=key, duration=duration,
                track=track, start=start, end=end, audio=audio,
            )
        except Exception as e:  # reported to the app as the failure message
            raise HTTPException(status_code=500, detail=f"{type(e).__name__}: {e}") from e
        return Response(content=wav, media_type="audio/wav")

    return app
