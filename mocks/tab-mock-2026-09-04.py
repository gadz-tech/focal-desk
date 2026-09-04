# Vector-studio mock rev 5 (12:43 pin: notification box smaller, clear of the corner arcs; 12:40 pins: notification boxes INSIDE the glass; stage border just thick enough for the landings): focal-desk frames.
# Screen mock: 1 unit = 1 px of a 1920x1080 view of the 8K panel.
def rr(x,y,w,h,r):
    """Rounded-rect: data-vs model (pts + fillets) and a derived d."""
    pts=f"{x},{y} {x+w},{y} {x+w},{y+h} {x},{y+h}"
    fil=",".join(f"{i}:{r}" for i in range(4))
    d=(f"M {x+r},{y} L {x+w-r},{y} A {r} {r} 0 0 1 {x+w},{y+r} L {x+w},{y+h-r} "
       f"A {r} {r} 0 0 1 {x+w-r},{y+h} L {x+r},{y+h} A {r} {r} 0 0 1 {x},{y+h-r} "
       f"L {x},{y+r} A {r} {r} 0 0 1 {x+r},{y} Z")
    return pts,fil,d

def hole(cx,cy,lit,c="#2FE6FF"):
    """Connection hole: neon when a wire connects this app to another, silver ring when free."""
    if lit: return f'  <circle cx="{cx:.0f}" cy="{cy:.0f}" r="7" fill="{c}" fill-opacity="0.9" stroke="#E8FBFF" stroke-width="1.5" filter="url(#neon)"/>\n'
    return f'  <circle cx="{cx:.0f}" cy="{cy:.0f}" r="7" fill="none" stroke="#C9CDD2" stroke-opacity="0.75" stroke-width="2"/>\n'

def frame(name,x,y,w,h,slim,thick,side,r_out=20,r_in=12,ports=True,lit_end=1,foot=30):
    """Frame = a data-vs rounded rect BEHIND the window rect; side = the thick edge (faces screen center).
    Vertical thick side: strip centred left-right, 2 holes at the ends, one-line notification box under the frame.
    Horizontal thick side: strip offset toward the window, notification box under the strip, 2 holes at the ends."""
    fx,fy,fw,fh=x-slim,y-slim,w+2*slim,h+2*slim
    if side=='L': fx-=thick-slim; fw+=thick-slim
    if side=='R': fw+=thick-slim
    if side=='T': fy-=thick-slim; fh+=thick-slim
    if side=='B': fh+=thick-slim
    if ports and side in 'LR': fh+=foot          # foot band under the window: the one-line box lives inside the glass
    p,f,d=rr(fx,fy,fw,fh,r_out)
    pw,fw_,dw=rr(x,y,w,h,r_in)
    s=f'  <!-- {name}: frame (behind), fade, window (on top), then the thick-side furniture -->\n'
    # frosted glass: one smooth gradient, no sheen, no horizon line (pin 10)
    s+=(f'  <path data-vs="1" data-vs-pts="{p}" data-vs-closed="1" data-vs-fillets="{f}" '
        f'd="{d}" fill="url(#frost)" stroke="#FFFFFF" stroke-opacity="0.22" stroke-width="1.2"/>\n')
    # inner edge fades into the window (pin 1 of rev 1)
    s+=f'  <rect x="{x-2}" y="{y-2}" width="{w+4}" height="{h+4}" rx="{r_in+2}" fill="#14140F" fill-opacity="0.85" filter="url(#fade)"/>\n'
    s+=(f'  <path data-vs="1" data-vs-pts="{pw}" data-vs-closed="1" data-vs-fillets="{fw_}" '
        f'd="{dw}" fill="#23221E" stroke="#3A3934" stroke-width="1"/>\n')
    if not ports: return s,None,None
    note=lambda bx,by,bw,bh: (f'  <rect x="{bx:.0f}" y="{by:.0f}" width="{bw:.0f}" height="{bh:.0f}" rx="6" fill="#FFFFFF" fill-opacity="0.06" stroke="#FFFFFF" stroke-opacity="0.18" stroke-width="1"/>\n'
                              f'  <text x="{bx+10:.0f}" y="{by+bh-7:.0f}" font-family="Segoe UI, Inter, sans-serif" font-size="12" fill="#F4F0E7" fill-opacity="0.35">notification · one line · later</text>\n')
    if side in 'LR':
        px = fx+thick/2 if side=='L' else fx+fw-thick/2      # centre of the thick band
        py = y+h/2
        s+=f'  <rect x="{px-1.5:.0f}" y="{py-100}" width="3" height="200" rx="1.5" fill="#2FE6FF" filter="url(#neon)"/>\n'   # centred (pin 1)
        ends=[(px,y+30),(px,y+h-30)]                              # connectors at the ends (pins 2,3,4)
        for i,(hx,hy) in enumerate(ends): s+=hole(hx,hy,i==lit_end)
        s+=note(x+15,y+h+slim+5,w-30,foot-10)                     # smaller, clear of the frame's corner arcs (rev 5 pin: no intersecting radii)
        port=ends[lit_end]; land_axis='h'
    else:
        px = x+w/2
        band_top = fy+fh-thick if side=='B' else fy
        sy_ = band_top+9 if side=='B' else band_top+thick-9        # strip offset toward the window (pin 7)
        s+=f'  <rect x="{px-100}" y="{sy_-1.5:.0f}" width="200" height="3" rx="1.5" fill="#2FE6FF" filter="url(#neon)"/>\n'
        by = sy_+7 if side=='B' else band_top+5
        bh = (fy+fh-5-by) if side=='B' else (sy_-7-by)
        s+=note(px-100,by,200,bh)                                 # box under the strip, INSIDE the band with margin (rev 4)
        hy = band_top+thick/2
        ends=[(x+30,hy),(x+w-30,hy)]                              # connectors at the ends (pins 8,9)
        for i,(hx,hy_) in enumerate(ends): s+=hole(hx,hy_,i==lit_end)
        port=ends[lit_end]; land_axis='v'
    return s,port,land_axis

W,H=1920,1080
out=[f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}">',
f'''  <!-- vector-studio mock rev 3 · focal-desk frames · 2026-09-04 · screen mock, 1 unit = 1 px of a 1920x1080 view of the 8K panel
       Brief (Ryan): frame encapsulates the whole window; frosted glass, rounded; Vista Aero brought to now with a hint of neon-1980s in
       the wires; slim ×3, thick toward center; frames BEHIND their window; stage = thin border, no tab.
       Rev 2 pins: inner edge fades into the window; strip long+thin; connection holes neon (wired) / silver (free); wires land on the stage frame.
       Rev 3 pins: strip centred on vertical sides, offset on horizontal; holes at the ENDS of the thick side (2); one-line notification box
       (under the frame on vertical sides, under the strip on horizontal); healthy gap top window ↔ stage; no sheen/horizon line. -->
  <defs>
    <linearGradient id="frost" x1="0" y1="0" x2="0.3" y2="1">
      <stop offset="0" stop-color="#E4EEF7" stop-opacity="0.30"/>
      <stop offset="0.5" stop-color="#C2D2E0" stop-opacity="0.22"/>
      <stop offset="1" stop-color="#93A9BE" stop-opacity="0.16"/>
    </linearGradient>
    <radialGradient id="stagehalo" cx="0.5" cy="0.5" r="0.7">
      <stop offset="0" stop-color="#E6A23C" stop-opacity="0.10"/>
      <stop offset="1" stop-color="#E6A23C" stop-opacity="0"/>
    </radialGradient>
    <filter id="fade" x="-10%" y="-10%" width="120%" height="120%"><feGaussianBlur stdDeviation="9"/></filter>
    <filter id="neon" x="-100%" y="-100%" width="300%" height="300%">
      <feGaussianBlur stdDeviation="7" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="b"/><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <filter id="wireglow" x="-10%" y="-40%" width="120%" height="180%">
      <feGaussianBlur stdDeviation="5" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
  </defs>
  <rect data-vs-bg="1" x="0" y="0" width="{W}" height="{H}" fill="#14140F"/>
  <g stroke="#FFFFFF" stroke-opacity="0.04" stroke-width="1">
    <line x1="520" y1="0" x2="520" y2="{H}"/><line x1="1400" y1="0" x2="1400" y2="{H}"/>
    <line x1="0" y1="120" x2="{W}" y2="120"/><line x1="0" y1="960" x2="{W}" y2="960"/>
  </g>''']
SF=18                                       # stage border: just thick enough for the landing holes (rev 4)
sx,sy,sw,sh=560,164,800,776                 # stage down again → 30 px gap to the top window's frame
out.append(f'  <rect x="{sx-120}" y="{sy-120}" width="{sw+240}" height="{sh+240}" fill="url(#stagehalo)"/>')
s,_,_=frame("stage",sx,sy,sw,sh,slim=SF,thick=SF,side='R',r_out=22,r_in=12,ports=False); out.append(s)
s,pL,_ =frame("left",60,200,400,680,slim=8,thick=42,side='R',lit_end=1);   out.append(s)
s,pTR,_=frame("topright",1440,60,420,340,slim=8,thick=42,side='L',lit_end=1); out.append(s)
s,pBR,_=frame("botright",1440,680,420,340,slim=8,thick=42,side='L',lit_end=0); out.append(s)
s,pT,_ =frame("top",600,8,720,66,slim=8,thick=42,side='B',lit_end=0);    out.append(s)   # frame bottom = 116; stage frame top = 149

def wire(p,q,c="#2FE6FF"):
    (x1,y1),(x2,y2)=p,q
    return (f'  <path d="M {x1:.0f},{y1:.0f} L {x2:.0f},{y2:.0f}" fill="none" stroke="{c}" stroke-width="2" stroke-opacity="0.9" filter="url(#wireglow)"/>')
def land(x,y,c="#2FE6FF"): return f'  <circle cx="{x:.0f}" cy="{y:.0f}" r="6" fill="{c}" fill-opacity="0.9" stroke="#E8FBFF" stroke-width="1" filter="url(#neon)"/>'
# wires: from the lit hole straight into the gutter, landing on the stage's slim frame (never over a window)
out.append(wire(pL,(sx-SF,pL[1])));                 out.append(land(sx-SF/2,pL[1]))
out.append(wire(pTR,(sx+sw+SF,pTR[1])));            out.append(land(sx+sw+SF/2,pTR[1]))
out.append(wire(pBR,(sx+sw+SF,pBR[1]),"#FF3FA4"));  out.append(land(sx+sw+SF/2,pBR[1],"#FF3FA4"))
out.append(wire(pT,(pT[0],sy-SF)));                 out.append(land(pT[0],sy-SF/2))
L=lambda x,y,t,a="start",c="#F4F0E7",o=0.8,sz=15: f'  <text x="{x}" y="{y}" font-family="Segoe UI, Inter, sans-serif" font-size="{sz}" fill="{c}" fill-opacity="{o}" text-anchor="{a}">{t}</text>'
out.append(L(sx+sw/2,sy+sh/2-10,"STAGE",'middle',"#E6A23C",0.9,26))
out.append(L(sx+sw/2,sy+sh/2+16,"border all round, just wide enough for the landings · no tab · it already has focus",'middle',"#F4F0E7",0.55))
out.append(L(260,540,"side window",'middle',"#F4F0E7",0.55))
out.append(L(260,562,"frame BEHIND the window · slim ×3 · thick toward center",'middle',"#F4F0E7",0.4,13))
out.append(L(1650,230,"terminal — corner slot",'middle',"#F4F0E7",0.5))
out.append(L(1650,850,"mail — bottom slot",'middle',"#F4F0E7",0.5))
out.append(L(960,48,"household — top slot",'middle',"#F4F0E7",0.5))
out.append(L(70,1040,"thick side: strip centred (vertical) / offset toward the window (horizontal) · holes at the ENDS — neon when wired, silver when free · wires land on the stage border",'start',"#F4F0E7",0.45,13))
out.append(L(70,1062,"frosted = one gradual gradient, no sheen, no horizon line (real build: DWM acrylic) · notification box INSIDE the glass: foot band (vertical) / under the strip (horizontal) — later",'start',"#F4F0E7",0.35,12))
out.append('</svg>')
svg="\n".join(out)
root='/sessions/serene-sweet-carson/mnt/box/focal-desk-2026-09-04/'
open(root+'tab-mock-2026-09-04.svg','w',encoding='utf-8').write(svg)
import cairosvg
cairosvg.svg2png(bytestring=svg.encode(),write_to=root+'tab-mock-2026-09-04.png',output_width=1920)
print(len(svg),'chars; data-vs paths:',svg.count('data-vs="1"'), '| gap top→stage:', (sy-SF)-(8+66+42))
