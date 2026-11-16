// SPDX-License-Identifier: Apache-2.0

// A static file server for the package, its probe page and test fixtures.
//
//   node scripts/serve.mjs [--port 8787] [--host 127.0.0.1] [--mount /prefix=dir]...
//                          [--cert cert.pem --key key.pem]
//
// `--host 0.0.0.0` serves the local network too, for a phone on the same
// Wi-Fi; the addresses to open are printed. A phone needs HTTPS for WebGPU
// and AudioWorklet, which browsers keep to secure contexts: give a
// certificate with --cert and --key (docs/web.md makes one with openssl).
// Once listening it prints one JSON line: {"ok":true,"url":...,"lan":[...]}.

import { createReadStream, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { createServer as createTlsServer } from "node:https";
import { networkInterfaces } from "node:os";
import { dirname, extname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".wasm": "application/wasm",
  ".png": "image/png",
  ".ttf": "font/ttf",
  ".css": "text/css; charset=utf-8",
  ".ts": "text/plain; charset=utf-8",
};

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
let port = 8787;
let host = "127.0.0.1";
let cert = null;
let key = null;
const mounts = [];
for (let i = 0; i < argv.length; i++) {
  const [flag, value] = [argv[i], argv[i + 1]];
  if (flag === "--port") port = Number(value);
  else if (flag === "--host") host = value;
  else if (flag === "--cert") cert = readFileSync(value);
  else if (flag === "--key") key = readFileSync(value);
  else if (flag === "--mount") {
    const at = value.indexOf("=");
    mounts.push([value.slice(0, at).replace(/\/?$/, "/"), resolve(value.slice(at + 1))]);
  } else {
    console.error(JSON.stringify({ ok: false, error: { code: "E_USAGE", message: `unknown argument ${flag}` } }));
    process.exit(2);
  }
  i++;
}
mounts.push(["/", pkg]);
mounts.sort((a, b) => b[0].length - a[0].length);

function locate(pathname) {
  for (const [prefix, dir] of mounts) {
    if (!pathname.startsWith(prefix)) continue;
    const file = resolve(join(dir, pathname.slice(prefix.length)));
    if (file !== dir && !file.startsWith(dir + sep)) return null;
    return file;
  }
  return null;
}

const handle = (req, res) => {
  const url = new URL(req.url ?? "/", "http://localhost");
  let file = locate(decodeURIComponent(url.pathname));
  let stat;
  try {
    stat = file && statSync(file);
    if (stat?.isDirectory()) {
      file = join(file, "index.html");
      stat = statSync(file);
    }
  } catch {
    stat = null;
  }
  if (!file || !stat) {
    res.writeHead(404, { "content-type": "text/plain" }).end("not found");
    return;
  }
  res.writeHead(200, {
    "content-type": TYPES[extname(file)] ?? "application/octet-stream",
    "content-length": stat.size,
    "cache-control": "no-store",
  });
  if (req.method === "HEAD") res.end();
  else createReadStream(file).pipe(res);
};
const scheme = cert && key ? "https" : "http";
const server = scheme === "https" ? createTlsServer({ cert, key }, handle) : createServer(handle);

server.listen(port, host, () => {
  const { port: bound } = server.address();
  const lan =
    host === "0.0.0.0"
      ? Object.values(networkInterfaces())
          .flat()
          .filter((a) => a && a.family === "IPv4" && !a.internal)
          .map((a) => `${scheme}://${a.address}:${bound}/`)
      : [];
  const shown = host === "0.0.0.0" ? "127.0.0.1" : host;
  console.log(JSON.stringify({ ok: true, url: `${scheme}://${shown}:${bound}/`, lan, pid: process.pid }));
});
