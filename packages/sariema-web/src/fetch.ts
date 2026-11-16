// SPDX-License-Identifier: Apache-2.0

export interface Progress {
  url: string;
  loaded: number;
  /** Zero while the length is not known. */
  total: number;
}

/**
 * The bytes at `url`, read as they stream in. With a known length they land
 * in one buffer of that size, so a pack of tens of megabytes is never held
 * twice while it arrives. The length is only a hint: across origins a
 * compressed response shows its compressed Content-Length but hides its
 * Content-Encoding, and the stream yields more bytes than that.
 */
export async function fetchBytes(url: string | URL, onProgress?: (p: Progress) => void): Promise<Uint8Array> {
  const href = String(url);
  const io = (e: unknown) => new Error(`E_IO: ${href}: ${e instanceof Error ? e.message : String(e)}`);
  const response = await fetch(href).catch((e) => {
    throw io(e);
  });
  if (!response.ok) throw new Error(`E_IO: ${href}: ${response.status} ${response.statusText}`);
  const encoded = response.headers.get("content-encoding");
  const length = encoded && encoded !== "identity" ? 0 : Number(response.headers.get("content-length") ?? 0);
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    onProgress?.({ url: href, loaded: bytes.length, total: bytes.length });
    return bytes;
  }
  let out = new Uint8Array(length || 1 << 16);
  let loaded = 0;
  const reader = response.body.getReader();
  for (;;) {
    const { done, value } = await reader.read().catch((e) => {
      throw io(e);
    });
    if (done) break;
    if (loaded + value.length > out.length) {
      const grown = new Uint8Array(Math.max(out.length * 2, loaded + value.length));
      grown.set(out.subarray(0, loaded));
      out = grown;
    }
    out.set(value, loaded);
    loaded += value.length;
    onProgress?.({ url: href, loaded, total: loaded <= length ? length : 0 });
  }
  return loaded === out.length ? out : out.slice(0, loaded);
}
