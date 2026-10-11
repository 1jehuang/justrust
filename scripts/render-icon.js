#!/usr/bin/env node
// Render the justrust icon: one still frame of the halftone gear from
// assets/gear.js (the jcode.sh/rust hero animation), written as SVG.
// It runs gear.js unchanged against a stub canvas that records every dot.
//
//   node scripts/render-icon.js [size] [t] > assets/justrust-icon.svg
"use strict";
const fs = require("fs");
const path = require("path");

const size = Number(process.argv[2] || 280);
const t = Number(process.argv[3] || 1.2);

const dots = [];
let fill = "#111";
const ctx = {
  scale() {}, clearRect() {}, beginPath() {}, moveTo() {}, fill() {},
  set fillStyle(v) { fill = v; }, get fillStyle() { return fill; },
  arc(x, y, r) { dots.push([x, y, r]); },
};
const canvas = {
  width: size, height: size, style: {},
  getContext: () => ctx, addEventListener() {}, setPointerCapture() {},
};
global.window = {
  devicePixelRatio: 1,
  // Reduced motion makes gear.js draw a single still frame at t=1.2.
  matchMedia: () => ({ matches: true }),
};
Object.defineProperty(global, "navigator", { value: {}, configurable: true });
global.document = { getElementById: () => canvas, addEventListener() {} };
global.performance = { now: () => 0 };

let src = fs.readFileSync(path.join(__dirname, "../assets/gear.js"), "utf8");
if (t !== 1.2) src = src.replace("draw(1.2)", `draw(${t})`);
eval(src);

const f = (n) => +n.toFixed(2);
const body = dots.map(([x, y, r]) => `<circle cx="${f(x)}" cy="${f(y)}" r="${f(r)}"/>`).join("");
process.stdout.write(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}">` +
  `<title>justrust</title><g fill="${fill}">${body}</g></svg>\n`
);
