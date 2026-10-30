// The compiler: `ra_jit_wasm.wasm` with its exports as methods.

export class Compiler {
  // `wasm` is the compiler's module, `core` the bundle of `core`'s source.
  static async load(wasm, core) {
    if (!(wasm instanceof WebAssembly.Module)) {
      wasm = await WebAssembly.compile(wasm);
    }
    let memory;
    const text = (pointer, length) =>
      new TextDecoder().decode(new Uint8Array(memory.buffer, pointer >>> 0, length >>> 0).slice());
    // What the compiler needs of the page: a clock, and somewhere to say why it panicked.
    const ra-jit = {
      now: () => performance.now(),
      log: (pointer, length) => console.error(text(pointer, length)),
    };
    const instance = await WebAssembly.instantiate(wasm, { ra-jit });
    memory = instance.exports.memory;
    const compiler = new Compiler(instance.exports);
    if (core && !compiler.ra-jit.ra_jit_add_bundle(...compiler.argument(core))) {
      throw new Error(compiler.report());
    }
    return compiler;
  }

  constructor(ra-jit) {
    this.ra-jit = ra-jit;
  }

  // Copies bytes or text into the compiler, which frees them once it has read them.
  argument(value) {
    const bytes = typeof value === "string" ? new TextEncoder().encode(value) : value;
    const pointer = this.ra-jit.ra_jit_alloc(bytes.length);
    new Uint8Array(this.ra-jit.memory.buffer, pointer >>> 0, bytes.length).set(bytes);
    return [pointer, bytes.length];
  }

  // Sets a packed cargo workspace as what there is to compile.
  setPack(pack) {
    if (!this.ra-jit.ra_jit_set_pack(...this.argument(pack))) {
      throw new Error(this.report());
    }
  }

  // Where in the module the pack is linked into the program may put its functions and data.
  setStart(tableStart, memoryStart) {
    this.ra-jit.ra_jit_set_start(tableStart, memoryStart);
  }

  analyze() {
    this.ra-jit.ra_jit_analyze();
    return this.report();
  }

  setFile(path, text) {
    this.ra-jit.ra_jit_set_file(...this.argument(path), ...this.argument(text));
  }

  report() {
    const { memory, ra_jit_report, ra_jit_report_len } = this.ra-jit;
    const bytes = new Uint8Array(memory.buffer, ra_jit_report() >>> 0, ra_jit_report_len());
    return new TextDecoder().decode(bytes.slice());
  }

  // Builds what changed. Gives the module to instantiate and the build's report, or throws
  // with what the compiler said.
  build() {
    if (!this.ra-jit.ra_jit_build()) {
      throw new Error(this.report());
    }
    const { memory, ra_jit_module, ra_jit_module_len } = this.ra-jit;
    const module = new Uint8Array(memory.buffer, ra_jit_module() >>> 0, ra_jit_module_len()).slice();
    return { module, report: JSON.parse(this.report()) };
  }

  // How much memory the compiler has.
  memoryBytes() {
    return this.ra-jit.memory.buffer.byteLength;
  }
}
