import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("./r2-data.js", import.meta.url), "utf8");
const { default: worker } = await import(`data:text/javascript,${encodeURIComponent(source)}`);
const path = "/data/generated/github-com-madler-zlib/0123456789abcdef0123456789abcdef01234567/data.pack";

function object(body, range) {
  return {
    body,
    range,
    size: 100,
    httpEtag: '"etag"',
    writeHttpMetadata(headers) {
      headers.set("cache-control", "public, max-age=31536000, immutable");
      headers.set("content-type", "application/octet-stream");
    }
  };
}

test("returns an R2 byte range with HTTP metadata", async () => {
  const env = {
    DATA: {
      async get(key, options) {
        assert.equal(key, path.slice("/data/".length));
        assert.equal(options.range.get("range"), "bytes=10-19");
        return object(new Uint8Array(10), { offset: 10, length: 10 });
      }
    }
  };
  const request = new Request(`https://code.dataisland.org${path}`, {
    headers: { range: "bytes=10-19" }
  });
  const result = await worker.fetch(request, env);

  assert.equal(result.status, 206);
  assert.equal(result.headers.get("content-range"), "bytes 10-19/100");
  assert.equal(result.headers.get("content-length"), "10");
  assert.equal(result.headers.get("accept-ranges"), "bytes");
  assert.equal(result.headers.get("etag"), '"etag"');
  assert.equal((await result.arrayBuffer()).byteLength, 10);
});

test("supports HEAD without reading the object body", async () => {
  const env = { DATA: { head: async () => object(undefined, undefined) } };
  const result = await worker.fetch(new Request(`https://code.dataisland.org${path}`, {
    method: "HEAD"
  }), env);

  assert.equal(result.status, 200);
  assert.equal(result.headers.get("content-length"), "100");
  assert.equal((await result.arrayBuffer()).byteLength, 0);
});

test("rejects unrelated paths, methods, and malformed ranges", async () => {
  const env = { DATA: {} };
  assert.equal((await worker.fetch(new Request("https://code.dataisland.org/data/private"), env)).status, 404);
  assert.equal((await worker.fetch(new Request(`https://code.dataisland.org${path}`, { method: "POST" }), env)).status, 405);
  assert.equal((await worker.fetch(new Request(`https://code.dataisland.org${path}`, {
    headers: { range: "bytes=20-10" }
  }), env)).status, 416);
});
