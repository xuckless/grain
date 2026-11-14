"""Command line: `serve` runs the HTTP API until the app stops it; `download` fetches the
ACE-Step weights the sidecar uses (the app's opt-in install runs it once).

Both run under `python -I` and exit when done: nothing stays resident between generations.
"""

import argparse
import os
import sys


def main(argv=None):
    p = argparse.ArgumentParser(prog="grain_sidecar")
    sub = p.add_subparsers(dest="command", required=True)
    serve = sub.add_parser("serve")
    serve.add_argument("--port", type=int, required=True)
    serve.add_argument("--backend", choices=["acestep", "stub"], default="acestep")
    serve.add_argument("--checkpoints", default=None)
    download = sub.add_parser("download")
    download.add_argument("--checkpoints", required=True)
    args = p.parse_args(argv)

    if args.command == "download":
        from .acestep_backend import download_weights

        download_weights(args.checkpoints)
        return 0

    token = os.environ.get("GRAIN_SIDECAR_TOKEN", "")
    if len(token) < 16:
        sys.exit("GRAIN_SIDECAR_TOKEN must be set (16+ characters)")
    import uvicorn

    from .server import create_app

    if args.backend == "stub":
        from .stub_backend import StubBackend

        backend = StubBackend()
    else:
        from .acestep_backend import AceStepBackend

        backend = AceStepBackend(args.checkpoints)
    # Loopback only: never reachable from other machines.
    uvicorn.run(create_app(backend, token), host="127.0.0.1", port=args.port, log_level="warning")
    return 0


if __name__ == "__main__":
    sys.exit(main())
