// SPDX-License-Identifier: GPL-3.0-only
// Run against actual wasm-bindgen --target web output, not a mocked analyzer.
// node crates/prophet-analysis/tests/web_runtime.mjs /absolute/generated/directory
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

const directory = resolve(process.argv[2]);
const { default: init, WebAnalyzer } = await import(pathToFileURL(resolve(directory, 'prophet_analysis.js')));
const wasmBytes = await readFile(resolve(directory, 'prophet_analysis_bg.wasm'));
const wasm = await init({ module_or_path: wasmBytes });
let scenarios = 0;
for (const n of [2048, 4096, 8192]) {
  for (const sampleRate of [44100, 48000, 96000]) {
    for (const window of [0, 1, 2]) {
      const analyzer = new WebAnalyzer(n, sampleRate, window, 1, 25, 1, 8.6);
      assert.equal(analyzer.bins(), n / 2 + 1);
      assert.equal(analyzer.sample_rate(), sampleRate);
      assert.equal(analyzer.fft_size(), n);
      assert.equal(analyzer.input_capacity(), 64);
      assert.ok(analyzer.coherent_gain() > 0 && analyzer.coherent_gain() <= 1);
      assert.ok(analyzer.enbw_bins() >= 1 && Number.isFinite(analyzer.enbw_hz()));
      if (window === 0) {
        assert.ok(Math.abs(analyzer.coherent_gain() - 0.5) < 1e-5);
        assert.ok(Math.abs(analyzer.enbw_bins() - 1.5) < 1e-5);
      }
      const pointers = [analyzer.input_ptr(), analyzer.db_ptr(), analyzer.peak_ptr()];
      const input = new Float32Array(wasm.memory.buffer, analyzer.input_ptr(), 64);
      const beforePackets = wasm.memory.buffer;
      for (let packet = 0; packet < n / 64; packet++) {
        for (let i = 0; i < 64; i++) input[i] = 0.5 * Math.sin(2 * Math.PI * 37 * (packet * 64 + i) / n);
        const emitted = analyzer.push(packet, packet * 64, 64);
        assert.equal(emitted, packet === n / 64 - 1 ? 1 : 0);
        assert.equal(wasm.memory.buffer, beforePackets, 'packet processing must not grow WASM memory');
      }
      assert.equal(analyzer.has_frame(), true);
      assert.equal(analyzer.first_frame(), 0);
      assert.equal(analyzer.end_frame(), n);
      assert.equal(analyzer.sequence(), 0);
      assert.equal(analyzer.discontinuities(), 0);
      const db = new Float32Array(wasm.memory.buffer, analyzer.db_ptr(), analyzer.bins());
      const peak = new Float32Array(wasm.memory.buffer, analyzer.peak_ptr(), analyzer.bins());
      assert.ok(Math.abs(db[37] + 6.0206) < 0.003, `window ${window}: -6dB sine reads ${db[37]}`);
      assert.equal(peak[37], db[37]);
      assert.deepEqual([analyzer.input_ptr(), analyzer.db_ptr(), analyzer.peak_ptr()], pointers);
      assert.equal(analyzer.push(n / 64 + 1, n + 64, 32), 0, 'short final chunks permitted');
      assert.equal(analyzer.has_frame(), false, 'gap invalidates old displayed frame');
      assert.equal(analyzer.discontinuities(), 1);
      assert.equal(analyzer.push(n / 64 + 1, n + 64, 32), -4, 'duplicate rejected');
      assert.equal(analyzer.push(-1, 0, 64), -2);
      assert.equal(analyzer.push(0.5, 0, 64), -2);
      assert.equal(analyzer.push(0, NaN, 64), -2);
      assert.equal(analyzer.push(0, Number.MAX_SAFE_INTEGER, 64), -2);
      assert.equal(analyzer.push(0, 0, 65), -1);
      analyzer.free();
      scenarios++;
    }
  }
}

const analyzer = new WebAnalyzer(2048, 48000, 0, 1, 25, 1, 8.6);
const staleView = new Float32Array(wasm.memory.buffer, analyzer.input_ptr(), 64);
wasm.memory.grow(1);
assert.equal(staleView.byteLength, 0, 'memory.grow detaches the prior JS view');
const input = new Float32Array(wasm.memory.buffer, analyzer.input_ptr(), 64);
input[0] = NaN;
assert.equal(analyzer.push(0, 0, 64), -3);
input.fill(0);
assert.equal(analyzer.push(0, 0, 64), 0, 'caller can refresh views and continue safely');
analyzer.free();
assert.throws(() => new WebAnalyzer(1024, 48000, 0, 1, 25, 1, 8.6));
console.log(JSON.stringify({ status: 'PASS', node: process.version, wasmScenarios: scenarios, wasmSha256: createHash('sha256').update(wasmBytes).digest('hex'), checks: ['actual Rust WASM at 2048/4096/8192 and 44.1/48/96kHz', 'Hann/BH/Kaiser coherent-gain amplitude and Rust window metrics', 'timestamps and sequence', 'stable pointers/no packet memory growth', 'short chunks, gaps and stale packets', 'invalid arguments and samples', 'typed-array refresh after memory.grow'] }));
