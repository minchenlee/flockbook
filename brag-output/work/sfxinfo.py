# Prints, for each wav: format, length, peak, loudness, time of the loudest 10 ms, and where the energy sits.
import sys, wave, glob, os, numpy as np
for f in sorted(glob.glob(os.path.join(sys.argv[1], '*.wav'))):
    w = wave.open(f); sr, ch, sw, n = w.getframerate(), w.getnchannels(), w.getsampwidth(), w.getnframes()
    raw = w.readframes(n); w.close()
    if sw == 2: x = np.frombuffer(raw, '<i2').astype(float) / 32768
    elif sw == 3:
        b = np.frombuffer(raw, np.uint8).reshape(-1, 3); x = (b[:, 0].astype(np.int32) | (b[:, 1].astype(np.int32) << 8) | (b[:, 2].astype(np.int8).astype(np.int32) << 16)) / 8388608
    else: x = np.frombuffer(raw, '<i4').astype(float) / 2**31
    x = x.reshape(-1, ch).mean(1)
    hop = int(sr * .01); e = np.array([np.sqrt(np.mean(x[i:i + hop] ** 2)) for i in range(0, len(x) - hop, hop)])
    on = np.argmax(e > e.max() * .1) * .01; pk = np.argmax(e) * .01; end = (len(e) - np.argmax(e[::-1] > e.max() * .05)) * .01
    sp = np.abs(np.fft.rfft(x * np.hanning(len(x)))); fr = np.fft.rfftfreq(len(x), 1 / sr); cen = (sp * fr).sum() / sp.sum(); dom = fr[np.argmax(sp)]
    print(f"{os.path.basename(f):16s} {sr}Hz {ch}ch {sw*8}bit len {n/sr:5.2f}s peak {np.abs(x).max():.2f} rms {np.sqrt(np.mean(x**2)):.3f} onset {on:.2f} loudest {pk:.2f} ends {end:.2f} centroid {cen:6.0f}Hz dominant {dom:6.0f}Hz")
