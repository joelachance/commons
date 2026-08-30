#!/usr/bin/env python3
"""Parakeet ASR sidecar.

Speaks newline-delimited JSON over stdin/stdout so the model stays resident and
is loaded exactly once:

    ->  {"path": "/abs/path/audio.wav"}
    <-  {"ok": true, "text": "..."}

stdout is protocol only. Everything human-readable goes to stderr.
"""

import json
import os
import sys

import numpy as np
import soundfile as sf
import soxr

MODEL = "mlx-community/parakeet-tdt-0.6b-v2"


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def load_mono(path, target_rate):
    """Reads any WAV, downmixes to mono, resamples to the model's rate."""
    audio, rate = sf.read(path, dtype="float32", always_2d=True)
    audio = audio.mean(axis=1)
    if rate != target_rate:
        audio = soxr.resample(audio, rate, target_rate, quality="HQ")
    return np.ascontiguousarray(audio, dtype=np.float32)


def load_model():
    """Loads from the local cache without touching the network when it can.

    The default path asks the Hub for the current revision on every start,
    which costs a round trip and fails outright on a plane. Offline first,
    online only as the fallback that populates the cache in the first place.
    """
    from parakeet_mlx import from_pretrained

    prior = os.environ.get("HF_HUB_OFFLINE")
    os.environ["HF_HUB_OFFLINE"] = "1"
    try:
        return from_pretrained(MODEL)
    except Exception as e:
        log(f"cache miss ({e}); fetching from the Hub")
        if prior is None:
            del os.environ["HF_HUB_OFFLINE"]
        else:
            os.environ["HF_HUB_OFFLINE"] = prior
        return from_pretrained(MODEL)


def main():
    import mlx.core as mx
    from parakeet_mlx.audio import get_logmel

    log(f"loading {MODEL} ...")
    model = load_model()
    rate = model.preprocessor_config.sample_rate
    log(f"ready (sample_rate={rate})")

    print(json.dumps({"ready": True, "model": MODEL}), flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            audio = load_mono(req["path"], rate)
            # Parakeet on a sub-second clip has nothing to work with, and the
            # mel front end wants at least a frame or two.
            if audio.size < rate // 10:
                resp = {"ok": True, "text": ""}
            else:
                mel = get_logmel(mx.array(audio), model.preprocessor_config)
                resp = {"ok": True, "text": model.generate(mel)[0].text}
        except Exception as e:  # a bad take must not take the sidecar down
            log(f"error: {e}")
            resp = {"ok": False, "error": str(e)}
        print(json.dumps(resp), flush=True)


if __name__ == "__main__":
    main()
