"""A stand-in backend for tests and development: no model, no weights. It answers with a
click track at the requested tempo, so the app's whole path (process, token, HTTP, WAV, new
track) can be exercised without downloading ACE-Step."""

import io
import math
import struct
import wave

RATE = 44100


class StubBackend:
    name = "stub"

    def device(self):
        return "cpu"

    def generate(self, *, task, prompt, seed, bpm, key, duration, track, start, end, audio):
        frames = int(duration * RATE)
        beat = int(RATE * 60.0 / bpm)
        tone = 440.0 + seed % 220
        samples = bytearray()
        for i in range(frames):
            t = i % beat
            v = 0.3 * math.sin(2 * math.pi * tone * i / RATE) * math.exp(-t / (0.03 * RATE))
            samples += struct.pack("<hh", int(v * 32767), int(v * 32767))
        out = io.BytesIO()
        with wave.open(out, "wb") as w:
            w.setnchannels(2)
            w.setsampwidth(2)
            w.setframerate(RATE)
            w.writeframes(bytes(samples))
        return out.getvalue()
