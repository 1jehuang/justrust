#!/usr/bin/env node
// Render the justrust icon: one still frame of the halftone gear from
// assets/gear.js (the jcode.sh/rust hero animation, copied unchanged from
// jcode-website/public/gear.js), written as SVG.
//
// It runs gear.js's CPU renderer against a stub canvas and records every
// halftone dot instead of rasterizing it, then crops the SVG to the gear so
// the icon is centered with an even margin.
//
//   node scripts/render-icon.js 0.6 0.35 48 > assets/justrust-icon.svg
//   INK=#e6e6e6 node scripts/render-icon.js 0.6 0.35 48 > assets/justrust-icon-dark.svg
//
// t is the animation time in seconds (spin), tilt overrides the tilt-back
// angle in radians (the hero swings between 0.66 and 0.90), and cells is
// the halftone lattice pitch (the hero uses 76; fewer cells give the bigger
// dots of the jcode mark, which hold up at icon sizes without moire).
"use strict";
const fs = require("fs");
const path = require("path");

const t = Number(process.argv[2] || 1.2);
const tilt = process.argv[3];
const cells = process.argv[4];
const ink = process.env.INK || "#111"; // INK=#e6e6e6 for the dark-theme variant
const size = 280; // the hero canvas size; dots are in its coordinates

const dots = [];
const ctx = {
  createImageData: (w, h) => ({ data: new Uint8ClampedArray(w * h * 4) }),
  putImageData() {},
};
const canvas = {
  width: size, height: size, style: {}, dataset: {},
  getContext: (kind) => (kind === "2d" ? ctx : null),
  addEventListener() {}, setPointerCapture() {},
};
global.window = {
  devicePixelRatio: 1,
  // Reduced motion makes gear.js draw a single still frame at draw(1.2).
  matchMedia: () => ({ matches: true }),
};
global.location = { search: "?gear=cpu" };
Object.defineProperty(global, "navigator", { value: {}, configurable: true });
global.document = { getElementById: () => canvas, addEventListener() {} };
global.performance = { now: () => 0 };
global.__dot = (x, y, r) => dots.push([x, y, r]);

let src = fs.readFileSync(path.join(__dirname, "../assets/gear.js"), "utf8");
const hook = "disc(px * dpr, py * dpr, ";
if (!src.includes(hook)) throw new Error("gear.js changed: dot call not found");
src = src.replace(hook, "__dot(px, py, ");
if (t !== 1.2) src = src.replace("draw(1.2)", `draw(${t})`);
if (tilt !== undefined) {
  const a = "var A = 0.78 + Math.sin(t * 0.4) * 0.12;";
  if (!src.includes(a)) throw new Error("gear.js changed: tilt not found");
  src = src.replace(a, `var A = ${Number(tilt)};`);
}
if (cells !== undefined) {
  if (!src.includes("var cells = 76;")) throw new Error("gear.js changed: cells not found");
  src = src.replace("var cells = 76;", `var cells = ${Number(cells)};`);
}
eval(src);
if (!dots.length) throw new Error("no dots rendered");

// Crop to the dots plus a margin, as a square centered on the gear.
let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
for (const [x, y, r] of dots) {
  if (r < 0.6) continue; // faint anti-aliased fringe does not set the bounds
  x0 = Math.min(x0, x - r); y0 = Math.min(y0, y - r);
  x1 = Math.max(x1, x + r); y1 = Math.max(y1, y + r);
}
const side = Math.max(x1 - x0, y1 - y0) * 1.06;
const vx = (x0 + x1) / 2 - side / 2, vy = (y0 + y1) / 2 - side / 2;

const f = (n) => +n.toFixed(2);
const body = dots.map(([x, y, r]) => `<circle cx="${f(x)}" cy="${f(y)}" r="${f(r)}"/>`).join("");
process.stdout.write(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${f(vx)} ${f(vy)} ${f(side)} ${f(side)}" width="512" height="512">` +
  `<title>justrust</title><g fill="${ink}">${body}</g></svg>\n`
);
