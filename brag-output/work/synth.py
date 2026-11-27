# Soundtrack for the Flockbook brag: 120 BPM, A minor / C major, 24 s. Music and effects share one beat grid.
import numpy as np, wave
SR=44100; DUR=24.0; BEAT=0.5; N=int(SR*DUR)
rng=np.random.default_rng(3)
L=np.zeros(N); R=np.zeros(N)
def add(sig,t,gain=1.0,pan=0.0):
    i=int(round(t*SR)); 
    if i>=N: return
    sig=sig[:N-i]
    L[i:i+len(sig)]+=sig*gain*(1-max(pan,0)*0.6); R[i:i+len(sig)]+=sig*gain*(1+min(pan,0)*0.6)
def env(n,a,d,curve=4.0):
    t=np.arange(n)/SR; e=np.exp(-t*curve/d); e[:max(1,int(a*SR))]*=np.linspace(0,1,max(1,int(a*SR))); 
    fade=min(n,200); e[-fade:]*=np.linspace(1,0,fade); return e
def lp(x,fc):
    a=1-np.exp(-2*np.pi*fc/SR); y=np.empty_like(x); z=0.0
    for i in range(len(x)): z+=a*(x[i]-z); y[i]=z
    return y
def hp(x,fc): return x-lp(x,fc)
f=lambda m: 440*2**((m-69)/12)
def kick():
    n=int(.32*SR); t=np.arange(n)/SR; ph=2*np.pi*(48*t+ (150-48)*0.035*(1-np.exp(-t/0.035)))
    return np.sin(ph)*env(n,.001,.22,5)
def hat(d=.04):
    n=int(d*2*SR); return hp(rng.standard_normal(n),7000)*env(n,.001,d,6)
def clap():
    n=int(.18*SR); x=hp(lp(rng.standard_normal(n),5000),900); e=env(n,.002,.11,5)
    return x*e
def pluck(m,d=.28,bright=1.0):
    n=int(d*2.2*SR); t=np.arange(n)/SR; fr=f(m)
    x=np.sin(2*np.pi*fr*t)+.35*bright*np.sin(2*np.pi*2*fr*t)*np.exp(-t*9)+.15*bright*np.sin(2*np.pi*3*fr*t)*np.exp(-t*14)
    return x*env(n,.004,d,4.5)
def bass(m,d=.42):
    n=int(d*SR); t=np.arange(n)/SR; fr=f(m)
    x=np.sin(2*np.pi*fr*t)+.3*np.sin(2*np.pi*2*fr*t)+.12*np.sin(2*np.pi*3*fr*t)
    e=np.ones(n); a=int(.006*SR); e[:a]=np.linspace(0,1,a); r=int(.08*SR); e[-r:]=np.linspace(1,0,r)
    return np.tanh(x*1.4)*e
def pad(notes,d):
    n=int(d*SR); t=np.arange(n)/SR; x=np.zeros(n)
    for m in notes:
        for det in (-.004,.004): x+=np.sin(2*np.pi*f(m)*(1+det)*t)+.2*np.sin(2*np.pi*2*f(m)*(1+det)*t)
    e=np.ones(n); a=int(.25*SR); e[:a]=np.linspace(0,1,a)**2; r=int(.5*SR); e[-r:]*=np.linspace(1,0,r)
    return x/len(notes)*e
def whoosh(d=.45):
    n=int(d*SR); x=rng.standard_normal(n); x=hp(lp(x,3200),500); e=np.linspace(0,1,n)**2.5; e[-300:]*=np.linspace(1,0,300); return x*e
def blip(m,d=.09):
    n=int(d*2.5*SR); t=np.arange(n)/SR; return np.sin(2*np.pi*f(m)*t)*env(n,.003,d,5)
def tick():
    n=int(.02*SR); return hp(rng.standard_normal(n),3000)*env(n,.0005,.008,5)

# bars of 2 s. chords: (root for bass, pad notes, arp notes)
Am=(45,[57,60,64,67],[69,72,76,79]); F=(41,[57,60,65,69],[69,72,77,81]); C=(48,[55,60,64,67],[67,72,76,79]); G=(43,[55,59,62,67],[67,71,74,79])
bars=[Am,Am, F,G, Am,C, F,G, Am,C, F,C]      # 12 bars = 24 s
for b,(root,pn,ar) in enumerate(bars):
    t0=b*2.0
    full = b>=2 and b<10; outro = b>=10; hook = b<2
    add(pad(pn,2.05),t0,0.05 if not hook else 0.035)
    for k in range(4):                      # beats
        tb=t0+k*BEAT
        if full or (outro and b==10): add(kick(),tb,0.55)
        if hook and k in (0,): add(kick(),tb,0.35)
        if full and k in (1,3): add(clap(),tb,0.10,0.1)
        if not outro or b==10:
            add(hat(),tb+.25,0.045 if not hook else 0.03,-0.2)
            if full: add(hat(.025),tb,0.02,0.25); add(hat(.02),tb+.375,0.015,0.3)
    if full or b==10:                       # bass: root on 1, push on the and-of-2, fifth on 4
        add(bass(root,.45),t0,0.26); add(bass(root,.3),t0+.75,0.22); add(bass(root+12,.2),t0+1.25,0.14); add(bass(root+7,.4),t0+1.5,0.22)
    if hook:                                # sparse ticking arp for tension
        for k in range(8): add(pluck(ar[0] if k%2==0 else ar[2],.12,.5),t0+k*.25,0.05,(-1)**k*.4)
    if full:
        pat=[0,2,1,3,2,1,3,2]
        for k,p in enumerate(pat): add(pluck(ar[p],.2),t0+k*.25,0.085,(-1)**k*.35)
    if b==10:
        for k,p in enumerate([0,1,2,3]): add(pluck(ar[p],.3),t0+k*.5,0.08,(-1)**k*.3)
# final chord rings out
add(pad([48,55,60,64,67,72],2.0),22.0,0.07); add(pluck(84,.9),22.0,0.07); add(pluck(79,.9),22.0,0.06,.3); add(kick(),22.0,0.5); add(bass(36,1.2),22.0,0.22)

# effects: samples from the user's own sound pack (work/sfx, not committed), each placed so its loudest moment
# lands on the event, on the same beat grid as the music
import os
def sample(name):
    w=wave.open(os.path.join(os.path.dirname(os.path.abspath(__file__)),'sfx44',name+'.wav')); x=np.frombuffer(w.readframes(w.getnframes()),'<i2').astype(float)/32768; w.close()
    return x.reshape(-1,2)
PEAK={'swoop':.25,'drawer':.17,'swish':.07,'whip':.10,'arrive':.25,'bloom':.09,'emblem':.05}
def sfx(name,t,gain=1.0,pan=0.0):
    x=sample(name); i=int(round((t-PEAK.get(name,0.0))*SR))
    if i<0: x=x[-i:]; i=0
    x=x[:N-i]
    L[i:i+len(x)]+=x[:,0]*gain*(1-max(pan,0)*0.5); R[i:i+len(x)]+=x[:,1]*gain*(1+min(pan,0)*0.5)
sfx('hint',2.0,.55)                                   # the prompt appears
sfx('swoop',4.0,.40); sfx('emblem',4.1,.75)            # into the logo
sfx('swish',7.0,.40)                                  # the window rises
sfx('appear',7.5,.30)
for k in range(5): sfx('tab',7.75+k*.125,.13,(-1)**k*.3)   # tool calls open
sfx('appear',8.5,.14)
sfx('dot',10.75,.45,-.2); sfx('dot',11.25,.55,.2)      # two more agents wait
sfx('tab',12.0,.50); sfx('popover',12.25,.55)          # tab click, approval card
sfx('thock',13.0,.45); sfx('check',13.0,.55); sfx('glide',13.25,.20)   # Allow once
for k in range(12): sfx('keycap' if k%3 else 'chiclet',14.25+k*.0625,.15+.05*(k%2),(-1)**k*.25)   # typing
sfx('enter',15.0,.40); sfx('dispatch',15.0,.50); sfx('bubble',15.125,.22); sfx('appear',15.5,.14)
sfx('tab',16.5,.45); sfx('thock',17.0,.35); sfx('drawer',17.125,.55,.3); sfx('tally',17.25,.30,.3)   # workspace, Resources
sfx('swoop',19.125,.45)                                # the phone rises
for k in range(5): sfx('stack',19.25+k*.125,.20,(-1)**k*.2)
sfx('swish',20.5,.35); sfx('arrive',21.0,.55); sfx('bloom',21.5,.30)

mix=np.stack([L,R],1)
mix=np.tanh(mix*1.5)/np.tanh(1.5)
fin=int(.02*SR); mix[:fin]*=np.linspace(0,1,fin)[:,None]
fo=int(1.2*SR); mix[-fo:]*=(np.linspace(1,0,fo)**1.5)[:,None]
mix=mix/np.max(np.abs(mix))*0.84
print('peak',np.max(np.abs(mix)),'rms',np.sqrt(np.mean(mix**2)))
w=wave.open('music.wav','wb'); w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR); w.writeframes((mix*32767).astype('<i2').tobytes()); w.close()
