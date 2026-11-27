# Tempo, beat phase and section energy of a track (mono 22.05 kHz wav), with numpy only.
import sys, wave, numpy as np
w = wave.open(sys.argv[1]); sr = w.getframerate(); x = np.frombuffer(w.readframes(w.getnframes()), '<i2').astype(float) / 32768; w.close()
hop = 256; n = 1024; fr = sr / hop
frames = np.lib.stride_tricks.sliding_window_view(x, n)[::hop] * np.hanning(n)
S = np.abs(np.fft.rfft(frames, axis=1)); logS = np.log1p(50 * S)
flux = np.maximum(0, np.diff(logS, axis=0)).sum(1); flux = flux - np.convolve(flux, np.ones(43) / 43, 'same'); flux = np.maximum(flux, 0)
# tempo: autocorrelation of the onset envelope, 70..180 BPM
ac = np.correlate(flux, flux, 'full')[len(flux) - 1:]
cands = []
for bpm in np.arange(70, 180.01, 0.05):
    lag = 60 / bpm * fr; i = int(lag); f = lag - i
    v = sum((ac[int(k * lag)] * (1 - (k * lag - int(k * lag))) + ac[int(k * lag) + 1] * (k * lag - int(k * lag))) for k in (1, 2, 4)) 
    cands.append((v, bpm))
cands.sort(reverse=True); best = cands[0][1]
print('tempo candidates:', [round(b, 2) for v, b in cands[:1]], 'others:', sorted({round(b) for v, b in cands[:400]}))
# refine against a long comb, then beat phase
def score(bpm, ph):
    t = np.arange(ph, len(flux) / fr, 60 / bpm); idx = (t * fr).astype(int); idx = idx[idx < len(flux)]; return flux[idx].sum() / len(idx)
bb = max(((max(score(b, p) for p in np.arange(0, 60 / b, 0.01)), b) for b in np.arange(best - 1, best + 1, 0.01)))[1]
ph = max((score(bb, p), p) for p in np.arange(0, 60 / bb, 0.002))[1]
print(f'refined tempo {bb:.2f} BPM, beat {60/bb:.4f}s, first beat at {ph:.3f}s')
# energy per bar (4 beats), full band and below 150 Hz
bar = 4 * 60 / bb; freqs = np.fft.rfftfreq(n, 1 / sr); low = S[:, freqs < 150].sum(1); full = S.sum(1)
print('bar  start   level  bass  onsets')
t = ph; k = 0
while t + bar < len(x) / sr:
    a, b = int(t * fr), int((t + bar) * fr)
    print(f'{k:3d} {t:6.2f}  {20*np.log10(full[a:b].mean()+1e-9):6.1f} {20*np.log10(low[a:b].mean()+1e-9):6.1f} {flux[a:b].mean():6.2f}')
    t += bar; k += 1
