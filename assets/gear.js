// /rust hero: a halftone 3D gear, drawn the same way as the homepage donut
// (public/donut.js): every cell of a luminance grid raymarches an SDF and
// the field is screened as a 45-degree rotated-dot halftone. The gear is a
// thick ring with trapezoid teeth and a hub, lit like donut.c.

(function () {
  "use strict";

  var TEETH = 12;
  var R_OUT = 2.05;    // ring outer radius (tooth roots)
  var R_IN = 1.25;     // ring inner radius
  var TOOTH = 0.42;    // tooth height
  var HALF_T = 0.5;    // half thickness
  var ROUND = 0.14;    // edge rounding: gives the halftone highlights to shade
  var HUB_R = 0.42;    // hub radius
  var SPOKES = 5;
  var INV_TEETH_SEG = TEETH / (2 * Math.PI);
  var SPOKE_SEG = (2 * Math.PI) / SPOKES, INV_SPOKE_SEG = 1 / SPOKE_SEG;

  function gearSdf(x, y, z) {
    var r = Math.sqrt(x * x + y * y);
    var a = Math.atan2(y, x);
    // Teeth: angular square wave with sloped flanks.
    var ft = a * INV_TEETH_SEG;
    var u = ft - Math.floor(ft) - 0.5; // -0.5..0.5
    var tooth = Math.max(0, Math.min(1, (0.3 - Math.abs(u)) * 6));
    var outer = R_OUT + TOOTH * tooth;
    // Ring in the plane, then extrude.
    var d2 = Math.max(r - outer, R_IN - r);
    // Hub disc and spokes fill the middle.
    var hub = r - HUB_R;
    var fs = a * INV_SPOKE_SEG;
    var su = (fs - Math.floor(fs) - 0.5) * SPOKE_SEG;
    var spoke = Math.max(Math.abs(Math.sin(su) * r) - 0.17, r - R_IN - 0.05);
    d2 = Math.min(d2, hub, spoke);
    // Rounded slab: shrink by ROUND, then grow back (rounded edges).
    var dz = Math.abs(z) - (HALF_T - ROUND);
    d2 += ROUND;
    var ox = Math.max(d2, 0), oz = Math.max(dz, 0);
    return Math.min(Math.max(d2, dz), 0) + Math.sqrt(ox * ox + oz * oz) - ROUND;
  }

  function makeGear(n) {
    var lum = new Float32Array(n * n);
    var K2 = 5.6;
    var K1 = (n * K2 * 3) / (8 * 2.25);
    var BOUND = R_OUT + TOOTH + 0.1;
    var CC = K2 * K2 - BOUND * BOUND;
    // Light from the upper left, toward the scene.
    var LX = -0.45, LY = 0.62, LZ = -0.64;

    // Rays only need to march where the gear can be: the slab |z| <= HALF_T
    // inside the cylinder r <= BOUND. A small margin keeps the silhouette
    // coverage fade (minD < aa) identical to marching the full sphere.
    var ZLIM = HALF_T + 0.12, RLIM = BOUND;

    function render(t, spin) {
      var A = 0.78 + Math.sin(t * 0.4) * 0.12; // tilted back so the rim shows
      var B = t * 0.45 + spin;                 // spin about the axle
      var cA = Math.cos(A), sA = Math.sin(A), cB = Math.cos(B), sB = Math.sin(B);
      // local<-world: rotate about x by A, then about the gear axle (z) by B.
      var M00 = cB, M01 = sB * cA, M02 = sB * sA;
      var M10 = -sB, M11 = cB * cA, M12 = cB * sA;
      var M20 = 0, M21 = -sA, M22 = cA;
      var oX = M02 * -K2, oY = M12 * -K2, oZ = M22 * -K2;
      var oo = oX * oX + oY * oY;
      var idx = 0;
      for (var yc = 0; yc < n; yc++) {
        var pyw = -(yc + 0.5 - n / 2) / K1;
        for (var xc = 0; xc < n; xc++, idx++) {
          var pxw = (xc + 0.5 - n / 2) / K1;
          var inv = 1 / Math.sqrt(pxw * pxw + pyw * pyw + 1);
          var wx = pxw * inv, wy = pyw * inv, wz = inv;
          var dx = M00 * wx + M01 * wy + M02 * wz;
          var dy = M10 * wx + M11 * wy + M12 * wz;
          var dz = M20 * wx + M21 * wy + M22 * wz;
          var b = oX * dx + oY * dy + oZ * dz;
          var disc = b * b - CC;
          if (disc <= 0) { lum[idx] = 0; continue; }
          var sq = Math.sqrt(disc), s = Math.max(0, -b - sq), sExit = -b + sq;
          // Clip to the slab.
          if (dz > 1e-9 || dz < -1e-9) {
            var z0 = (-ZLIM - oZ) / dz, z1 = (ZLIM - oZ) / dz;
            if (z0 > z1) { var tz = z0; z0 = z1; z1 = tz; }
            if (z0 > s) s = z0;
            if (z1 < sExit) sExit = z1;
          } else if (oZ > ZLIM || oZ < -ZLIM) { lum[idx] = 0; continue; }
          // Clip to the cylinder.
          var qa = dx * dx + dy * dy;
          if (qa > 1e-12) {
            var qb = oX * dx + oY * dy, qd = qb * qb - qa * (oo - RLIM * RLIM);
            if (qd <= 0) { lum[idx] = 0; continue; }
            var qs = Math.sqrt(qd), c0 = (-qb - qs) / qa, c1 = (-qb + qs) / qa;
            if (c0 > s) s = c0;
            if (c1 < sExit) sExit = c1;
          }
          if (s >= sExit) { lum[idx] = 0; continue; }
          var hit = false, minD = 1e9, sMin = s;
          for (var i = 0; i < 64 && s < sExit; i++) {
            var d = gearSdf(oX + dx * s, oY + dy * s, oZ + dz * s);
            if (d < minD) { minD = d; sMin = s; }
            if (d < 1e-3) { hit = true; break; }
            s += d * 0.9;
          }
          var sp = hit ? s : sMin;
          var aa = (1.5 * sp) / K1;
          if (!hit && minD >= aa) { lum[idx] = 0; continue; }
          var px = oX + dx * sp, py = oY + dy * sp, pz = oZ + dz * sp, e = 0.004;
          // Tetrahedral gradient: 4 SDF calls instead of 6.
          var k0 = gearSdf(px + e, py - e, pz - e);
          var k1 = gearSdf(px - e, py - e, pz + e);
          var k2 = gearSdf(px - e, py + e, pz - e);
          var k3 = gearSdf(px + e, py + e, pz + e);
          var nx = k0 - k1 - k2 + k3;
          var ny = -k0 - k1 + k2 + k3;
          var nz = -k0 + k1 - k2 + k3;
          var nl = Math.sqrt(nx * nx + ny * ny + nz * nz) || 1;
          nx /= nl; ny /= nl; nz /= nl;
          // world normal = transpose(local<-world) * n
          var nwx = M00 * nx + M10 * ny + M20 * nz;
          var nwy = M01 * nx + M11 * ny + M21 * nz;
          var nwz = M02 * nx + M12 * ny + M22 * nz;
          var v = nwx * LX + nwy * LY + nwz * LZ;
          // Lambert plus a little ambient: the face, rim and rounded edges
          // land on different dot sizes, so the gear reads as solid.
          v = 0.06 + 0.78 * Math.max(0, v);
          if (!hit) v *= 1 - minD / aa;
          lum[idx] = v;
        }
      }
      return lum;
    }
    return { lum: lum, render: render };
  }

  function start(canvas) {
    var cells = 76;
    var constrained = (navigator.hardwareConcurrency && navigator.hardwareConcurrency <= 4) ||
      (navigator.deviceMemory && navigator.deviceMemory <= 4);
    // Same governor as the homepage donut: the dot lattice never changes,
    // only the luminance field behind it steps down when frames run long.
    var qualitySteps = constrained ? [56, 72, 84] : [56, 72, 84, 100, 120];
    var qualityIndex = qualitySteps.length - 1, grid, gear;
    function setQuality(i) {
      qualityIndex = Math.max(0, Math.min(i, qualitySteps.length - 1));
      grid = qualitySteps[qualityIndex];
      gear = makeGear(grid);
    }
    setQuality(qualityIndex);
    var ctx = canvas.getContext("2d");
    var cssW = canvas.width, cssH = canvas.height;
    var dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = cssW * dpr; canvas.height = cssH * dpr;
    canvas.style.width = cssW + "px"; canvas.style.maxWidth = "100%"; canvas.style.height = "auto";
    var W = canvas.width, H = canvas.height;
    var image = ctx.createImageData(W, H);
    var px32 = new Uint32Array(image.data.buffer);
    // #111 ink as little-endian RGBA, alpha in the top byte.
    var INK = 0x111111, INK_SOLID = (0xff000000 | INK) >>> 0;
    var s = cssW / cells, half = cssW / 2;
    var cosA = Math.cos(Math.PI / 4), sinA = Math.sin(Math.PI / 4);
    var ext = Math.ceil(cells * 0.7071) + 1;

    function sample(gx, gy) {
      if (gx < 0 || gy < 0 || gx > grid - 2 || gy > grid - 2) return 0;
      var x0 = gx | 0, y0 = gy | 0, fx = gx - x0, fy = gy - y0, l = gear.lum;
      return l[x0 + y0 * grid] * (1 - fx) * (1 - fy) + l[x0 + 1 + y0 * grid] * fx * (1 - fy) +
        l[x0 + (y0 + 1) * grid] * (1 - fx) * fy + l[x0 + 1 + (y0 + 1) * grid] * fx * fy;
    }

    var dragVel = 0, dragging = false, lastX = 0, spin = 0;
    canvas.style.cursor = "grab";
    canvas.style.touchAction = "pan-y";
    canvas.addEventListener("pointerdown", function (e) {
      dragging = true; lastX = e.clientX; canvas.setPointerCapture(e.pointerId); canvas.style.cursor = "grabbing";
    });
    canvas.addEventListener("pointermove", function (e) {
      if (!dragging) return; dragVel += (e.clientX - lastX) * 0.008; lastX = e.clientX;
    });
    var end = function () { dragging = false; canvas.style.cursor = "grab"; };
    canvas.addEventListener("pointerup", end);
    canvas.addEventListener("pointercancel", end);

    function draw(t) {
      spin += dragVel; dragVel *= 0.92;
      gear.render(t, spin);
      // Rasterize the dots straight into pixels: ~2,700 anti-aliased discs
      // cost far less written by hand than as one giant arc path, which made
      // canvas fill() the most expensive part of every frame.
      px32.fill(0);
      for (var j = -ext; j <= ext; j++) {
        for (var i = -ext; i <= ext; i++) {
          var px = half + (i * cosA - j * sinA) * s;
          var py = half + (i * sinA + j * cosA) * s;
          if (px < -s || px > cssW + s || py < -s || py > cssH + s) continue;
          var v = sample((px / cssW) * grid, (py / cssH) * grid);
          if (v <= 0.04) continue;
          disc(px * dpr, py * dpr, Math.pow(v, 0.85) * s * 0.62 * dpr);
        }
      }
      ctx.putImageData(image, 0, 0);
    }

    // One disc with 1px coverage anti-aliasing, composited "over" in ink.
    function disc(cx, cy, r) {
      var x0 = Math.max(0, Math.floor(cx - r)), x1 = Math.min(W - 1, Math.ceil(cx + r));
      var y0 = Math.max(0, Math.floor(cy - r)), y1 = Math.min(H - 1, Math.ceil(cy + r));
      var rIn = r - 0.5, rIn2 = rIn > 0 ? rIn * rIn : 0, rOut2 = (r + 0.5) * (r + 0.5);
      for (var y = y0; y <= y1; y++) {
        var dy = y + 0.5 - cy, dy2 = dy * dy, row = y * W;
        for (var x = x0; x <= x1; x++) {
          var dx = x + 0.5 - cx, d2 = dx * dx + dy2;
          if (d2 >= rOut2) continue;
          var k = row + x;
          if (d2 <= rIn2) { px32[k] = INK_SOLID; continue; }
          var cov = r + 0.5 - Math.sqrt(d2);
          if (cov > 1) cov = 1;
          var a = px32[k] >>> 24;
          a = a + ((255 - a) * cov + 0.5) | 0;
          px32[k] = ((a << 24) | INK) >>> 0;
        }
      }
    }

    var reduced = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) { draw(1.2); return; }
    var visible = true, t0 = performance.now(), raf = 0, last = 0;
    var renderMs = 0, slow = 0, fast = 0;
    function frame(ts) {
      raf = 0;
      if (!visible || document.hidden) return;
      if (ts - last >= 33) {
        last = ts;
        var before = performance.now();
        draw((ts - t0) / 1000);
        var cost = performance.now() - before;
        renderMs = renderMs ? renderMs * 0.9 + cost * 0.1 : cost;
        if (renderMs > 9 && qualityIndex > 0) {
          fast = 0;
          if (++slow >= 6) { setQuality(qualityIndex - 1); slow = 0; renderMs = 0; }
        } else if (renderMs < 4 && qualityIndex < qualitySteps.length - 1) {
          slow = 0;
          if (++fast >= 90) { setQuality(qualityIndex + 1); fast = 0; renderMs = 0; }
        } else {
          slow = Math.max(0, slow - 1); fast = 0;
        }
      }
      raf = requestAnimationFrame(frame);
    }
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (es) {
        visible = es.some(function (e) { return e.isIntersecting; });
        if (visible && !raf) raf = requestAnimationFrame(frame);
      }).observe(canvas);
    }
    document.addEventListener("visibilitychange", function () {
      if (!document.hidden && !raf) raf = requestAnimationFrame(frame);
    });
    draw(0);
    raf = requestAnimationFrame(frame);
  }

  // ---- GPU path (WebGL2) ----
  // Pass 1 raymarches the same SDF into a GRID x GRID luminance texture,
  // one fragment per cell, exactly what makeGear() does on the CPU.
  // Pass 2 screens it: each pixel finds its nearest dots on the 45-degree
  // lattice, bilinearly samples the texture at each dot center, and draws
  // the dot with ~1px coverage AA. The main thread only sets two uniforms.
  var GPU_GRID = 120;
  var VS = "#version 300 es\nvoid main(){vec2 p=vec2[3](vec2(-1,-1),vec2(3,-1),vec2(-1,3))[gl_VertexID];gl_Position=vec4(p,0,1);}";
  var FS_LUM = [
    "#version 300 es",
    "precision highp float;",
    "uniform mat3 u_m; uniform vec3 u_o; uniform float u_n;",
    "out vec4 o;",
    "const float PI=3.14159265359;",
    "const float TEETH=" + TEETH.toFixed(1) + ", R_OUT=" + R_OUT + ", R_IN=" + R_IN + ", TOOTH=" + TOOTH + ";",
    "const float HALF_T=" + HALF_T + ", ROUND=" + ROUND + ", HUB_R=" + HUB_R + ", SPOKES=" + SPOKES.toFixed(1) + ";",
    "const float K2=5.6; const float BOUND=R_OUT+TOOTH+0.1;",
    "float sdf(vec3 p){",
    "  float r=length(p.xy); float a=atan(p.y,p.x);",
    "  float ft=a*TEETH/(2.0*PI); float u=fract(ft)-0.5;",
    "  float tooth=clamp((0.3-abs(u))*6.0,0.0,1.0);",
    "  float d2=max(r-(R_OUT+TOOTH*tooth),R_IN-r);",
    "  float sseg=2.0*PI/SPOKES; float su=(fract(a/sseg)-0.5)*sseg;",
    "  float spoke=max(abs(sin(su)*r)-0.17,r-R_IN-0.05);",
    "  d2=min(d2,min(r-HUB_R,spoke));",
    "  float dz=abs(p.z)-(HALF_T-ROUND); d2+=ROUND;",
    "  return min(max(d2,dz),0.0)+length(max(vec2(d2,dz),0.0))-ROUND;",
    "}",
    "void main(){",
    "  float n=u_n; float K1=(n*K2*3.0)/(8.0*2.25);",
    "  vec2 c=gl_FragCoord.xy; float xc=c.x-0.5, yc=n-c.y-0.5;",
    "  vec3 w=normalize(vec3((xc+0.5-n*0.5)/K1,-(yc+0.5-n*0.5)/K1,1.0));",
    "  vec3 d=u_m*w; vec3 org=u_o;",
    "  float b=dot(org,d); float disc=b*b-(K2*K2-BOUND*BOUND);",
    "  if(disc<=0.0){o=vec4(0);return;}",
    "  float sq=sqrt(disc); float s=max(0.0,-b-sq), sE=-b+sq;",
    "  float ZL=HALF_T+0.12;",
    "  if(abs(d.z)>1e-9){float z0=(-ZL-org.z)/d.z,z1=(ZL-org.z)/d.z; s=max(s,min(z0,z1)); sE=min(sE,max(z0,z1));}",
    "  float qa=dot(d.xy,d.xy);",
    "  if(qa>1e-12){float qb=dot(org.xy,d.xy); float qd=qb*qb-qa*(dot(org.xy,org.xy)-BOUND*BOUND);",
    "    if(qd<=0.0){o=vec4(0);return;} float qs=sqrt(qd); s=max(s,(-qb-qs)/qa); sE=min(sE,(-qb+qs)/qa);}",
    "  if(s>=sE){o=vec4(0);return;}",
    "  bool hit=false; float minD=1e9, sMin=s;",
    "  for(int i=0;i<64;i++){ if(s>=sE) break; float dd=sdf(org+d*s);",
    "    if(dd<minD){minD=dd;sMin=s;} if(dd<1e-3){hit=true;break;} s+=dd*0.9; }",
    "  float sp=hit?s:sMin; float aa=1.5*sp/K1;",
    "  if(!hit&&minD>=aa){o=vec4(0);return;}",
    "  vec3 p=org+d*sp; const float e=0.004;",
    "  float k0=sdf(p+vec3(e,-e,-e)),k1=sdf(p+vec3(-e,-e,e)),k2=sdf(p+vec3(-e,e,-e)),k3=sdf(p+vec3(e,e,e));",
    "  vec3 nl=normalize(vec3(k0-k1-k2+k3,-k0-k1+k2+k3,-k0+k1-k2+k3));",
    "  vec3 nw=transpose(u_m)*nl;", // world normal: u_m is world->local
    "  float v=0.06+0.78*max(0.0,dot(nw,vec3(-0.45,0.62,-0.64)));",
    "  if(!hit) v*=1.0-minD/aa;",
    "  o=vec4(v,0,0,1);",
    "}"
  ].join("\n");
  var FS_DOTS = [
    "#version 300 es",
    "precision highp float;",
    "uniform sampler2D u_lum; uniform vec2 u_res; uniform float u_n; uniform float u_pitch;",
    "out vec4 o;",
    "const float C=0.7071067811865476;",
    "float lumAt(vec2 pt){",
    "  vec2 g=pt/u_res*u_n;",
    "  if(g.x<0.0||g.y<0.0||g.x>u_n-2.0||g.y>u_n-2.0) return 0.0;",
    // CPU samples cell (x0,y0) at index x0; texel centers sit at +0.5.
    "  return texture(u_lum,vec2(g.x+0.5,u_n-g.y-0.5)/u_n).r;",
    "}",
    "void main(){",
    "  vec2 sc=vec2(gl_FragCoord.x,u_res.y-gl_FragCoord.y); vec2 ce=sc-u_res*0.5;",
    "  vec2 l=vec2((ce.x+ce.y)*C,(-ce.x+ce.y)*C)/u_pitch; vec2 base=floor(l);",
    "  float a=0.0;",
    "  for(int y=0;y<=1;y++) for(int x=0;x<=1;x++){",
    "    vec2 cell=base+vec2(x,y);",
    "    vec2 pt=u_res*0.5+vec2((cell.x-cell.y)*C,(cell.x+cell.y)*C)*u_pitch;",
    "    float v=lumAt(pt); if(v<=0.04) continue;",
    "    float r=pow(v,0.85)*u_pitch*0.62;",
    "    float cov=clamp(r+0.5-distance(sc,pt),0.0,1.0);",
    "    a=a+(1.0-a)*cov;",
    "  }",
    "  o=vec4(vec3(0.0666667)*a,a);",
    "}"
  ].join("\n");

  function startGpu(canvas) {
    var gl = canvas.getContext("webgl2", { premultipliedAlpha: true, antialias: false, alpha: true });
    if (!gl || !gl.getExtension("EXT_color_buffer_float")) return false;
    function prog(fs) {
      var p = gl.createProgram();
      [[gl.VERTEX_SHADER, VS], [gl.FRAGMENT_SHADER, fs]].forEach(function (x) {
        var sh = gl.createShader(x[0]); gl.shaderSource(sh, x[1]); gl.compileShader(sh); gl.attachShader(p, sh);
      });
      gl.linkProgram(p);
      return gl.getProgramParameter(p, gl.LINK_STATUS) ? p : null;
    }
    var pLum = prog(FS_LUM), pDots = prog(FS_DOTS);
    if (!pLum || !pDots) return false;
    var N = GPU_GRID;
    var tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, N, N, 0, gl.RED, gl.HALF_FLOAT, null);
    // R16F is filterable in WebGL2, so the hardware does the bilinear sample.
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    var fb = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) return false;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.bindVertexArray(gl.createVertexArray());

    var cssW = canvas.width, cssH = canvas.height, cells = 76;
    var dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = cssW * dpr; canvas.height = cssH * dpr;
    canvas.style.width = cssW + "px"; canvas.style.maxWidth = "100%"; canvas.style.height = "auto";
    var uM = gl.getUniformLocation(pLum, "u_m"), uO = gl.getUniformLocation(pLum, "u_o");
    gl.useProgram(pLum); gl.uniform1f(gl.getUniformLocation(pLum, "u_n"), N);
    gl.useProgram(pDots);
    gl.uniform1i(gl.getUniformLocation(pDots, "u_lum"), 0);
    gl.uniform2f(gl.getUniformLocation(pDots, "u_res"), canvas.width, canvas.height);
    gl.uniform1f(gl.getUniformLocation(pDots, "u_n"), N);
    gl.uniform1f(gl.getUniformLocation(pDots, "u_pitch"), (cssW / cells) * dpr);
    gl.clearColor(0, 0, 0, 0);

    var spin = 0, dragVel = 0;
    function draw(t) {
      spin += dragVel; dragVel *= 0.92;
      var A = 0.78 + Math.sin(t * 0.4) * 0.12, B = t * 0.45 + spin;
      var cA = Math.cos(A), sA = Math.sin(A), cB = Math.cos(B), sB = Math.sin(B);
      // Same local<-world matrix as the CPU path, column-major for GLSL.
      var M = [cB, -sB, 0, sB * cA, cB * cA, -sA, sB * sA, cB * sA, cA];
      gl.useProgram(pLum);
      gl.uniformMatrix3fv(uM, false, M);
      gl.uniform3f(uO, sB * sA * -5.6, cB * sA * -5.6, cA * -5.6);
      gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
      gl.viewport(0, 0, N, N);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.useProgram(pDots);
      gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    }
    canvas.dataset.renderer = "gpu";
    return { draw: draw, drag: function (dv) { dragVel += dv; } };
  }

  var c = document.getElementById("gear-canvas");
  var want = (location.search.match(/[?&]gear=(cpu|gpu)/) || [])[1];
  var gpu = null;
  if (c && want !== "cpu") {
    try { gpu = startGpu(c); } catch (e) { gpu = null; }
  }
  if (gpu) runGpu(c, gpu);
  else if (c && c.getContext) { c.dataset.renderer = "cpu"; start(c); }

  function runGpu(canvas, r) {
    var dragging = false, lastX = 0;
    canvas.style.cursor = "grab";
    canvas.style.touchAction = "pan-y";
    canvas.addEventListener("pointerdown", function (e) {
      dragging = true; lastX = e.clientX; canvas.setPointerCapture(e.pointerId); canvas.style.cursor = "grabbing";
    });
    canvas.addEventListener("pointermove", function (e) {
      if (!dragging) return; r.drag((e.clientX - lastX) * 0.008); lastX = e.clientX;
    });
    var end = function () { dragging = false; canvas.style.cursor = "grab"; };
    canvas.addEventListener("pointerup", end);
    canvas.addEventListener("pointercancel", end);
    var reduced = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) { r.draw(1.2); return; }
    var visible = true, t0 = performance.now(), raf = 0, last = 0;
    function frame(ts) {
      raf = 0;
      if (!visible || document.hidden) return;
      if (ts - last >= 33) { last = ts; r.draw((ts - t0) / 1000); }
      raf = requestAnimationFrame(frame);
    }
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (es) {
        visible = es.some(function (e) { return e.isIntersecting; });
        if (visible && !raf) raf = requestAnimationFrame(frame);
      }).observe(canvas);
    }
    document.addEventListener("visibilitychange", function () {
      if (!document.hidden && !raf) raf = requestAnimationFrame(frame);
    });
    r.draw(0);
    raf = requestAnimationFrame(frame);
  }
})();
