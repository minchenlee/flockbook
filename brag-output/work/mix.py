# Soundtrack with a real track: python3 mix.py <track file> <start seconds> <bpm> <video seconds>
# The composition's timeline is written at 120 BPM; every event time is divided by bpm/120 so it stays on the beat.
import sys, os, subprocess, wave, numpy as np
track, start, bpm, dur = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
here = os.path.dirname(os.path.abspath(__file__)); SR = 44100; K = bpm / 120; N = int(SR * dur)
import json
TM = json.load(open(os.path.join(here, sys.argv[5]))) if len(sys.argv) > 5 else None
def to_video(t):   # composition time -> video seconds
    if not TM: return t / K
    p = TM['points']; i = 0
    while i < len(p) - 2 and t >= p[i + 1][1]: i += 1
    return (p[i][0] + (t - p[i][1]) * (p[i + 1][0] - p[i][0]) / (p[i + 1][1] - p[i][1])) * 60 / bpm
raw = subprocess.run(['ffmpeg', '-loglevel', 'error', '-ss', str(start), '-t', str(dur), '-i', track, '-f', 's16le', '-ac', '2', '-ar', str(SR), '-'], capture_output=True, check=True).stdout
m = np.frombuffer(raw, '<i2').astype(float).reshape(-1, 2) / 32768
m = np.vstack([m, np.zeros((max(0, N - len(m)), 2))])[:N]
m *= 0.17 / np.sqrt(np.mean(m ** 2))                      # the music's level; the effects sit under it
# hook: the first two bars are muffled, then the filter opens into the logo beat (bar 2)
t_open = 2 * 4 * 60 / bpm; n_open = int(t_open * SR); tt = np.arange(n_open) / SR
fc = np.where(tt < t_open * 0.72, 420.0, 420.0 * (20000 / 420.0) ** np.clip((tt - t_open * 0.72) / (t_open * 0.28), 0, 1))
al = 1 - np.exp(-2 * np.pi * fc / SR)
for ch in range(2):
    x = m[:n_open, ch].copy()
    for _ in range(3):
        y = np.empty_like(x); z = 0.0
        for i in range(n_open): z += al[i] * (x[i] - z); y[i] = z
        x = y
    m[:n_open, ch] = x * np.interp(tt, [0, t_open * 0.72, t_open], [1.5, 1.5, 1.0])
L = m[:, 0].copy(); R = m[:, 1].copy()
def sample(name):
    w = wave.open(os.path.join(here, 'sfx44', name + '.wav')); x = np.frombuffer(w.readframes(w.getnframes()), '<i2').astype(float) / 32768; w.close(); return x.reshape(-1, 2)
PEAK = {'swoop': .25, 'drawer': .17, 'swish': .07, 'whip': .10, 'arrive': .25, 'bloom': .09, 'emblem': .05}
def sfx(name, t, gain=1.0, pan=0.0):   # t is on the composition's timeline
    x = sample(name) ; i = int(round((to_video(t) - PEAK.get(name, 0.0)) * SR))
    if i < 0: x = x[-i:]; i = 0
    x = x[:N - i]
    L[i:i + len(x)] += x[:, 0] * gain * (1 - max(pan, 0) * 0.5); R[i:i + len(x)] += x[:, 1] * gain * (1 + min(pan, 0) * 0.5)
G = 1.15
sfx('hint', 2.0, .55 * G)
sfx('swoop', 4.0, .40 * G); sfx('emblem', 4.1, .55 * G)
sfx('swish', 7.0, .40 * G); sfx('appear', 7.5, .30 * G)
for k in range(5): sfx('tab', 7.75 + k * .125, .13 * G, (-1) ** k * .3)
sfx('appear', 8.5, .14 * G)
sfx('dot', 10.75, .45 * G, -.2); sfx('dot', 11.25, .55 * G, .2)
sfx('tab', 12.0, .50 * G); sfx('popover', 12.25, .55 * G)
sfx('thock', 13.0, .45 * G); sfx('check', 13.0, .55 * G); sfx('glide', 13.25, .20 * G)
for k in range(12): sfx('keycap' if k % 3 else 'chiclet', 14.25 + k * .0625, (.15 + .05 * (k % 2)) * G, (-1) ** k * .25)
sfx('enter', 15.0, .40 * G); sfx('dispatch', 15.0, .50 * G); sfx('bubble', 15.125, .22 * G); sfx('appear', 15.5, .14 * G)
sfx('tab', 16.5, .45 * G); sfx('thock', 17.0, .35 * G); sfx('drawer', 17.125, .55 * G, .3); sfx('tally', 17.25, .30 * G, .3)
sfx('swoop', 19.125, .45 * G)
for k in range(5): sfx('stack', 19.25 + k * .125, .20 * G, (-1) ** k * .2)
sfx('swish', 20.5, .35 * G); sfx('arrive', 21.0, .45 * G)
mix = np.stack([L, R], 1); mix = np.tanh(mix * 1.3) / np.tanh(1.3)
fi = int(.03 * SR); mix[:fi] *= np.linspace(0, 1, fi)[:, None]
fo = int(.9 * SR); mix[-fo:] *= (np.linspace(1, 0, fo) ** 1.5)[:, None]
mix = mix / np.max(np.abs(mix)) * 0.86
print('peak', round(float(np.max(np.abs(mix))), 2), 'rms', round(float(np.sqrt(np.mean(mix ** 2))), 3))
w = wave.open(os.path.join(here, 'music.wav'), 'wb'); w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR); w.writeframes((mix * 32767).astype('<i2').tobytes()); w.close()
