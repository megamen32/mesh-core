import http from 'node:http';

/** Same-user private Rust singleton. Caller retains authorization and discovery. */
export class MeshCoreClient {
  constructor(socketPath) {
    if (typeof socketPath !== 'string' || !socketPath.startsWith('/')) throw new Error('absolute core socket required');
    this.socketPath = socketPath;
  }
  async normalize(input) { return this.#call('/v1/normalize', input); }
  async topology(snapshot, now_ms) { return this.#call('/v1/topology', { snapshot, now_ms }); }
  #call(path, input) {
    // Copy before awaiting: no cross-call mutable profile or request state.
    const body = Buffer.from(JSON.stringify(input));
    if (body.length > 128 * 1024) throw new Error('bounded core request exceeded');
    return new Promise((resolve, reject) => {
      let response, size = 0, settled = false;
      const chunks = [];
      const finish = (error, value) => {
        if (settled) return;
        settled = true; clearTimeout(timer);
        request.destroy(); response?.destroy();
        error ? reject(error) : resolve(value);
      };
      const request = http.request({ socketPath: this.socketPath, method: 'POST', path, agent: false,
        headers: { 'Content-Type': 'application/json', 'Content-Length': body.length } }, incoming => {
        response = incoming;
        incoming.on('error', () => finish(new Error('core response failed')));
        incoming.on('aborted', () => finish(new Error('core response incomplete')));
        incoming.on('data', chunk => {
          size += chunk.length;
          if (size > 1024 * 1024) return finish(new Error('bounded core response exceeded'));
          chunks.push(chunk);
        });
        incoming.on('end', () => {
          if (incoming.statusCode !== 200) return finish(new Error('core request rejected'));
          try { finish(undefined, JSON.parse(Buffer.concat(chunks).toString('utf8'))); }
          catch { finish(new Error('invalid core response')); }
        });
      });
      const timer = setTimeout(() => finish(new Error('core deadline')), 3000);
      request.on('error', () => finish(new Error('core unavailable')));
      request.end(body);
    });
  }
}
