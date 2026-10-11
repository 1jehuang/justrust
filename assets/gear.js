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

  function gearSdf(x, y, z) {
    var r = Math.sqrt(x * x + y * y);
    var a = Math.atan2(y, x);
    // Teeth: angular square wave with sloped flanks.
    var seg = (2 * Math.PI) / TEETH;
    var u = ((a % seg) + seg) % seg / seg - 0.5; // -0.5..0.5
    var tooth = Math.max(0, Math.min(1, (0.3 - Math.abs(u)) * 6));
    var outer = R_OUT + TOOTH * tooth;
    // Ring in the plane, then extrude.
    var d2 = Math.max(r - outer, R_IN - r);
    // Hub disc and spokes fill the middle.
    var hub = r - HUB_R;
    var sseg = (2 * Math.PI) / SPOKES;
    var su = ((a % sseg) + sseg) % sseg - sseg / 2;
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

    function render(t, spin) {
      var A = 0.78 + Math.sin(t * 0.4) * 0.12; // tilted back so the rim shows
      var B = t * 0.45 + spin;                 // spin about the axle
      var cA = Math.cos(A), sA = Math.sin(A), cB = Math.cos(B), sB = Math.sin(B);
      // local<-world: rotate about x by A, then about the gear axle (z) by B.
      var M00 = cB, M01 = sB * cA, M02 = sB * sA;
      var M10 = -sB, M11 = cB * cA, M12 = cB * sA;
      var M20 = 0, M21 = -sA, M22 = cA;
      var oX = M02 * -K2, oY = M12 * -K2, oZ = M22 * -K2;
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
          var nx = gearSdf(px + e, py, pz) - gearSdf(px - e, py, pz);
          var ny = gearSdf(px, py + e, pz) - gearSdf(px, py - e, pz);
          var nz = gearSdf(px, py, pz + e) - gearSdf(px, py, pz - e);
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
    var cells = 76, grid = 120;
    var constrained = (navigator.hardwareConcurrency && navigator.hardwareConcurrency <= 4) ||
      (navigator.deviceMemory && navigator.deviceMemory <= 4);
    if (constrained) grid = 84;
    var gear = makeGear(grid);
    var ctx = canvas.getContext("2d");
    var cssW = canvas.width, cssH = canvas.height;
    var dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = cssW * dpr; canvas.height = cssH * dpr;
    canvas.style.width = cssW + "px"; canvas.style.maxWidth = "100%"; canvas.style.height = "auto";
    ctx.scale(dpr, dpr);
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
      ctx.clearRect(0, 0, cssW, cssH);
      ctx.fillStyle = "#111";
      ctx.beginPath();
      for (var j = -ext; j <= ext; j++) {
        for (var i = -ext; i <= ext; i++) {
          var px = half + (i * cosA - j * sinA) * s;
          var py = half + (i * sinA + j * cosA) * s;
          if (px < -s || px > cssW + s || py < -s || py > cssH + s) continue;
          var v = sample((px / cssW) * grid, (py / cssH) * grid);
          if (v <= 0.04) continue;
          var rad = Math.pow(v, 0.85) * s * 0.62;
          ctx.moveTo(px + rad, py);
          ctx.arc(px, py, rad, 0, Math.PI * 2);
        }
      }
      ctx.fill();
    }

    var reduced = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) { draw(1.2); return; }
    var visible = true, t0 = performance.now(), raf = 0, last = 0;
    function frame(ts) {
      raf = 0;
      if (!visible || document.hidden) return;
      if (ts - last >= 33) { last = ts; draw((ts - t0) / 1000); }
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

  var c = document.getElementById("gear-canvas");
  if (c && c.getContext) start(c);
})();
