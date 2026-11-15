// AudioWorkletGlobalScope lacks TextDecoder/TextEncoder in some browsers, but the wasm-bindgen
// glue creates both at module load. Imported before the glue so they exist when it evaluates.
// UTF-8 only; used for error strings.

const scope = globalThis as unknown as Record<string, unknown>;

if (!("TextDecoder" in scope)) {
  scope.TextDecoder = class {
    decode(bytes?: Uint8Array): string {
      if (!bytes) return "";
      let out = "";
      for (let i = 0; i < bytes.length; ) {
        const b = bytes[i++];
        let cp = b;
        if (b >= 0xf0) cp = ((b & 7) << 18) | ((bytes[i++] & 63) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63);
        else if (b >= 0xe0) cp = ((b & 15) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63);
        else if (b >= 0xc0) cp = ((b & 31) << 6) | (bytes[i++] & 63);
        out += String.fromCodePoint(cp);
      }
      return out;
    }
  };
}

if (!("TextEncoder" in scope)) {
  scope.TextEncoder = class {
    encode(text = ""): Uint8Array {
      const bytes: number[] = [];
      for (const ch of text) {
        const cp = ch.codePointAt(0)!;
        if (cp < 0x80) bytes.push(cp);
        else if (cp < 0x800) bytes.push(0xc0 | (cp >> 6), 0x80 | (cp & 63));
        else if (cp < 0x10000) bytes.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 63), 0x80 | (cp & 63));
        else bytes.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 63), 0x80 | ((cp >> 6) & 63), 0x80 | (cp & 63));
      }
      return new Uint8Array(bytes);
    }
    encodeInto(text: string, view: Uint8Array) {
      const bytes = this.encode(text);
      view.set(bytes.subarray(0, view.length));
      return { read: text.length, written: Math.min(bytes.length, view.length) };
    }
  };
}

export {};
