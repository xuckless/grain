// Single-producer single-consumer ring in a SharedArrayBuffer: the player worklet writes the
// microphone into it, the main thread drains it (AGENTS.md §9: no allocation or locks on the
// audio thread). Header (Int32): [0] samples ever written, [1] latest block peak (f32 bits),
// [2] samples dropped because the reader fell behind, [3] samples ever read.

const HEADER = 4;
/** About 47 s at 44.1 kHz; the main thread drains every ~50 ms. */
export const RING_SAMPLES = 1 << 21;

export type Ring = { header: Int32Array; data: Float32Array; level: Float32Array };

export function createRing(): SharedArrayBuffer {
  return new SharedArrayBuffer(HEADER * 4 + RING_SAMPLES * 4);
}

export function ringViews(buffer: SharedArrayBuffer): Ring {
  const header = new Int32Array(buffer, 0, HEADER);
  return { header, level: new Float32Array(buffer, 4, 1), data: new Float32Array(buffer, HEADER * 4, RING_SAMPLES) };
}

/** Audio thread: appends one block (mono; silence when the input isn't connected yet). */
export function writeBlock(ring: Ring, block: Float32Array | undefined, frames: number): void {
  let written = Atomics.load(ring.header, 0);
  const read = Atomics.load(ring.header, 3);
  let peak = 0;
  for (let i = 0; i < frames; i++) {
    const v = block ? block[i] : 0;
    peak = Math.max(peak, Math.abs(v));
    if (written - read >= RING_SAMPLES) {
      Atomics.add(ring.header, 2, 1);
      continue;
    }
    ring.data[written % RING_SAMPLES] = v;
    written++;
  }
  ring.level[0] = peak;
  Atomics.store(ring.header, 0, written);
}

/** Reader: everything written since the last drain, as one copy. */
export function drain(ring: Ring): Float32Array {
  const read = Atomics.load(ring.header, 3);
  const written = Atomics.load(ring.header, 0);
  const out = new Float32Array(written - read);
  for (let i = 0; i < out.length; i++) out[i] = ring.data[(read + i) % RING_SAMPLES];
  Atomics.store(ring.header, 3, written);
  return out;
}

export function dropped(ring: Ring): number {
  return Atomics.load(ring.header, 2);
}
