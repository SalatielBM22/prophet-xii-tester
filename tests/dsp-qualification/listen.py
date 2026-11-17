"""Generate two anonymized PCM files and a separate key for a human listening session."""
from pathlib import Path
import sys
import random
import json
import subprocess
import numpy as np
from scipy.io import wavfile

root, out = map(Path, sys.argv[1:])
out.mkdir(parents=True, exist_ok=True)
order = [0, 1]
random.SystemRandom().shuffle(order)
key = {}
for label, algorithm in zip("AB", order):
    source = root / f"audition-{algorithm}.f32"
    def loudness(path, raw):
        fmt = ["-f", "f32le", "-ar", "48000", "-ac", "2"] if raw else []
        proc = subprocess.run(["ffmpeg", "-hide_banner", "-nostats", *fmt, "-i", str(path),
                               "-af", "loudnorm=print_format=json", "-f", "null", "-"],
                              check=True, capture_output=True, text=True)
        return float(json.JSONDecoder().raw_decode(proc.stderr[proc.stderr.rindex("{"):])[0]["input_i"])
    measured = loudness(source, True)
    x = np.fromfile(source, dtype="<f4").reshape(-1, 2) * 10**((-23-measured)/20)
    assert np.isfinite(x).all() and np.abs(x).max() < 1
    path = out / f"{label}.wav"
    wavfile.write(path, 48000, np.rint(x*32767).astype("<i2"))
    actual = loudness(path, False)
    assert abs(actual+23)<0.15
    key[label] = {"algorithm":["LegacyRC1", "DiffuseV1"][algorithm], "LUFS":actual}
(out/"key.json").write_text(json.dumps(key, indent=2)+"\n")
