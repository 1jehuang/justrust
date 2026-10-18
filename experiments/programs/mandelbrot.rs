// SPDX-License-Identifier: MIT
// Standalone ASCII Mandelbrot renderer, written for this benchmark.
// Same source for jr-rust and rustc. No conditional compiler-specific paths.
// Fixed dimensions avoid unsupported command-line parsing and formatting APIs.

fn pixel(x: usize, y: usize) -> i32 {
    let cr = (x as f64) * 3.0 / 120.0 - 2.0;
    let ci = (y as f64) * 2.0 / 40.0 - 1.0;
    let mut zr: f64 = 0.0;
    let mut zi: f64 = 0.0;
    let mut iteration: i32 = 0;
    while zr * zr + zi * zi <= 4.0 && iteration < 200 {
        let next = zr * zr - zi * zi + cr;
        zi = 2.0 * zr * zi + ci;
        zr = next;
        iteration += 1;
    }
    iteration
}

fn emit_pixel(iteration: i32) {
    if iteration == 200 {
        print!("@");
    } else if iteration >= 32 {
        print!("#");
    } else if iteration >= 16 {
        print!("+");
    } else if iteration >= 8 {
        print!(".");
    } else {
        print!(" ");
    }
}

fn main() {
    let mut y: usize = 0;
    while y < 40 {
        let mut x: usize = 0;
        while x < 120 {
            emit_pixel(pixel(x, y));
            x += 1;
        }
        print!("\n");
        y += 1;
    }
}
