"""Offline metrics for the controlled signals emitted by the Rust probes."""
import csv
import sys
from pathlib import Path

import numpy as np
from scipy import signal

root, out = map(Path, sys.argv[1:])
out.mkdir(exist_ok=True, parents=True)


def dbc(value, reference):
    return 20 * np.log10(max(value / reference, 1e-15))


with (out / "drive.csv").open("w") as f:
    w = csv.writer(f)
    w.writerow("rate os model drive tones makeup rms sample_peak peak_4x_est dc thd_2_8_dbc imd3_dbc folded_h3_dbc".split())
    for path in sorted(root.glob("drive*.f32")):
        rate, os, model, drive, tones = map(int, path.stem.split("-")[1:])
        raw = np.fromfile(path, dtype="<f4").astype(float)
        for makeup in [False, True]:
            x = raw * (drive if makeup else 1)
            mag = np.abs(np.fft.rfft(x))
            # Periodic extension avoids edge transients for these coherent tones.
            interp = signal.resample_poly(np.tile(x, 3), 4, 1).reshape(3, -1)[1]
            fund = 1703 if tones == 3 else 64
            thd = dbc(np.linalg.norm(mag[np.arange(2, 9)*fund]), mag[fund]) if tones == 1 else ""
            imd = dbc(np.linalg.norm(mag[[21, 150]]), mag[fund]) if tones == 2 else ""
            fold = dbc(mag[len(x)-3*fund], mag[fund]) if tones == 3 else ""
            w.writerow([rate, os, ["Curtis","Moog"][model], drive, tones, int(makeup), np.mean(x*x)**0.5, max(abs(x)), max(abs(interp)), np.mean(x), thd, imd, fold])

with (out / "reverb.csv").open("w") as f:
    w = csv.writer(f)
    w.writerow("rate algorithm peak correlation edt_s t20_s t30_s clarity80_db end_rms".split())
    for path in sorted(root.glob("ir*.f32")):
        rate, alg = map(int, path.stem.split("-")[1:])
        x = np.fromfile(path, dtype="<f4").reshape(-1, 2).astype(float)
        energy = np.sum(x*x, axis=1)
        edc = np.cumsum(energy[::-1])[::-1]
        db = 10*np.log10(np.maximum(edc/edc[0], 1e-30))
        times = np.arange(len(x))/rate
        estimates = []
        for low, high in [(-10,0),(-25,-5),(-35,-5)]:
            indices = (db>=low)&(db<=high)
            slope, _ = np.polyfit(times[indices], db[indices], 1)
            estimates.append(-60/slope)
        cut = round(.08*rate)
        w.writerow([rate, ["LegacyRC1","DiffuseV1"][alg], np.max(abs(x)), np.corrcoef(x.T)[0,1], *estimates, 10*np.log10(energy[:cut].sum()/energy[cut:].sum()), np.mean(x[-rate//4:]**2)**0.5])
