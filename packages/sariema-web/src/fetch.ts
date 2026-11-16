// SPDX-License-Identifier: Apache-2.0

export interface Progress {
  url: string;
  loaded: number;
  /** Zero when the server sends no length. */
  total: number;
}

/**
 * The bytes at `url`, read as they stream in. With a known length they land
 * in one buffer of that size, so a pack of tens of megabytes is never held
 * twice while it arrives.
 */
export async function fetchBytes(url: string | URL, onProgress?: (p: Progress) => void): Promise<Uint8Array> {
  const href = String(url);
  const response = await fetch(href);
  if (!response.ok) throw new Error(`E_IO: ${href}: ${response.status} ${response.statusText}`);
  const total = Number(response.headers.get("content-length") ?? 0);
  const encoded = response.headers.get("content-encoding");
  if (!response.body || !total || (encoded && encoded !== "identity")) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    onProgress?.({ url: href, loaded: bytes.length, total: bytes.length });
    return bytes;
  }
  const out = new Uint8Array(total);
  const reader = response.body.getReader();
  let loaded = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    if (loaded + value.length > total) throw new Error(`E_IO: ${href}: more bytes than its length`);
    out.set(value, loaded);
    loaded += value.length;
    onProgress?.({ url: href, loaded, total });
  }
  if (loaded !== total) throw new Error(`E_IO: ${href}: ${loaded} of ${total} bytes arrived`);
  return out;
}
