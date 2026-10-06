// TanStack Start's prerenderer writes pages into `.output/public` AFTER the
// Nitro server bundle (and its inlined public-asset manifest) has been built,
// so the running server never serves prerendered pages statically — it
// re-renders them on every request instead. This rewrites the manifest inside
// `.output/server/index.mjs` from the final contents of `.output/public`, so
// prerendered HTML and the search index are served as static files.
//
// Run as part of `bun run build` (see package.json). Fails loudly if the
// nitro-generated markers change, so a nitro upgrade can't silently skip it.
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const publicDir = join(root, '.output/public');
const serverDir = join(root, '.output/server');
const entryPath = join(serverDir, 'index.mjs');

// Only applies to self-hosted presets (bun/node) where Nitro's inlined
// manifest is what serves static files. On Vercel the CDN serves the static
// output directly and this layout doesn't exist.
if (!existsSync(entryPath) || !existsSync(publicDir)) {
  console.log('[patch-asset-manifest] no bun/node server output, skipping');
  process.exit(0);
}

const MIME = {
  html: 'text/html; charset=utf-8',
  js: 'text/javascript; charset=utf-8',
  mjs: 'text/javascript; charset=utf-8',
  css: 'text/css; charset=utf-8',
  json: 'application/json; charset=utf-8',
  txt: 'text/plain; charset=utf-8',
  md: 'text/markdown; charset=utf-8',
  xml: 'application/xml; charset=utf-8',
  svg: 'image/svg+xml',
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  avif: 'image/avif',
  ico: 'image/x-icon',
  woff: 'font/woff',
  woff2: 'font/woff2',
  ttf: 'font/ttf',
  webmanifest: 'application/manifest+json',
  wasm: 'application/wasm',
};

async function collectFiles(dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await collectFiles(full)));
    else out.push(full);
  }
  return out;
}

function mimeType(id) {
  const ext = id.split('.').pop()?.toLowerCase() ?? '';
  return MIME[ext] ?? 'text/plain; charset=utf-8';
}

// Same shape the `etag` package produces: "<size hex>-<sha1 base64 prefix>".
function etagFor(data) {
  const hash = createHash('sha1').update(data).digest('base64').slice(0, 27);
  return `"${data.length.toString(16)}-${hash}"`;
}

const assets = {};
for (const file of (await collectFiles(publicDir)).sort()) {
  const id = `/${relative(publicDir, file).split('\\').join('/')}`;
  const [data, stats] = await Promise.all([readFile(file), stat(file)]);
  assets[id] = {
    type: mimeType(id),
    etag: etagFor(data),
    mtime: stats.mtime.toJSON(),
    size: stats.size,
    path: relative(serverDir, file),
  };
}

const entry = await readFile(entryPath, 'utf8');
const startMarker = '//#region #nitro/virtual/public-assets-data';
const endMarker = '//#endregion';
const start = entry.indexOf(startMarker);
if (start === -1) {
  throw new Error(`marker not found in ${entryPath}: ${startMarker}`);
}
const bodyStart = start + startMarker.length;
const end = entry.indexOf(endMarker, bodyStart);
if (end === -1) {
  throw new Error(`end marker not found in ${entryPath}`);
}

const replacement = `\nvar public_assets_data_default = ${JSON.stringify(assets, null, 2)};\n`;
await writeFile(
  entryPath,
  entry.slice(0, bodyStart) + replacement + entry.slice(end),
);

console.log(
  `[patch-asset-manifest] inlined ${Object.keys(assets).length} public assets into server manifest`,
);
