// Native-versus-WASM semantic-digest parity (PTE-NO-049, PTE-DET-004).
//
// `docs/IMPLEMENTATION_STATUS.md` recorded this as gap 4: `cargo check
// --target wasm32-unknown-unknown` proves the engine is host-free, but it does
// not prove the digests match, "because the conformance manifest has not been
// run in a browser or a JavaScript runtime".
//
// This runs it in one. The same `pte` front end is compiled for `wasm32-wasip1`
// and executed under Node's WASI, fed the same bytes as the native binary, and
// the two `semanticDigest` values are compared. A difference means the engine
// made a different decision on a different target, which §20.1 forbids.
//
// What this establishes, precisely: the engine's *arithmetic and decisions* are
// identical between a native x86-64 build and a wasm32 build executed by a
// JavaScript engine's WebAssembly implementation. What it does not establish:
// behaviour of a `wasm-bindgen` binding, which does not exist, or of a browser
// engine other than V8. Both are still open, and the status document says so.
//
// Usage:  node tools/wasm-parity.mjs <fixture.pam> [<fixture.pam> ...]
// Exit:   0 if every digest matches, 1 otherwise.

import { WASI } from 'node:wasi';
import { readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { openSync, closeSync, readFileSync, unlinkSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const engineRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const wasmPath = join(engineRoot, 'target/wasm32-wasip1/release/pte.wasm');
const nativePath = join(engineRoot, 'target/release/pte');

/** Run the native `pte inspect` and return its report. */
function native(fixture) {
  const stdout = execFileSync(nativePath, ['inspect', fixture], {
    maxBuffer: 64 * 1024 * 1024,
  });
  return JSON.parse(stdout.toString('utf8'));
}

/**
 * Run the wasm `pte inspect` under WASI and return its report.
 *
 * The report goes to stdout, which WASI cannot hand back as a buffer, so
 * stdout is redirected to a file and read afterwards. The fixture directory is
 * preopened read-only so the guest can reach it under the same path.
 */
async function wasm(module, fixture) {
  const scratch = mkdtempSync(join(tmpdir(), 'pte-parity-'));
  const outPath = join(scratch, 'stdout');
  const outFd = openSync(outPath, 'w');
  try {
    const wasi = new WASI({
      version: 'preview1',
      // argv[0] is the program name, as it is natively.
      args: ['pte', 'inspect', '/fixtures/' + fixture.name],
      env: {},
      preopens: { '/fixtures': fixture.dir },
      stdout: outFd,
      returnOnExit: true,
    });
    const instance = await WebAssembly.instantiate(
      module,
      wasi.getImportObject(),
    );
    const code = wasi.start(instance);
    if (code !== 0) {
      throw new Error(`the wasm build exited with ${code}`);
    }
  } finally {
    closeSync(outFd);
  }
  const text = readFileSync(outPath, 'utf8');
  try {
    unlinkSync(outPath);
  } catch {
    /* the scratch directory is disposable */
  }
  return JSON.parse(text);
}

const fixtures = process.argv.slice(2);
if (fixtures.length === 0) {
  console.error('usage: node tools/wasm-parity.mjs <fixture.pam> ...');
  process.exit(2);
}

const module = await WebAssembly.compile(await readFile(wasmPath));
let failures = 0;

for (const path of fixtures) {
  const absolute = resolve(path);
  const fixture = { dir: dirname(absolute), name: absolute.split('/').pop() };

  const a = native(absolute);
  const b = await wasm(module, fixture);

  const same = a.semanticDigest === b.semanticDigest;
  if (!same) {
    failures += 1;
  }
  console.log(
    `${same ? 'match  ' : 'DIFFER '} ${fixture.name}\n` +
      `  native ${a.semanticDigest}\n` +
      `  wasm   ${b.semanticDigest}`,
  );

  // A matching digest is the load-bearing claim, but a difference in the
  // measured geometry with the *same* digest would mean the digest is not
  // covering what it should. Both are checked.
  for (const [section, keys] of [
    ['topology', ['faces', 'sharedEdges', 'holes', 'junctions']],
    ['representation', ['lines', 'cubics', 'arcs', 'controlPoints', 'svgBytes']],
    ['boundarySources', ['coverageReconstructed', 'crispGrid', 'pixelArtPolicy']],
  ]) {
    for (const key of keys) {
      if (a[section]?.[key] !== b[section]?.[key]) {
        failures += 1;
        console.log(
          `  DIFFER ${section}.${key}: native ${a[section]?.[key]} vs wasm ${b[section]?.[key]}`,
        );
      }
    }
  }
}

if (failures > 0) {
  console.error(`\n${failures} parity failure(s)`);
  process.exit(1);
}
console.log(`\n${fixtures.length} fixture(s), native and wasm32 agree`);
