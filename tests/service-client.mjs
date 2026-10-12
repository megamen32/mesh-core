// Focused integration: real compiled Rust executable + real Node HTTP client.
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, chmod, lstat, rm, rmdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { MeshCoreClient } from '../bindings/node/client.mjs';

const directory = await mkdtemp(path.join(process.env.TMPDIR, 'rpc-'));
await chmod(directory, 0o700);
const socket = path.join(directory, 'core.sock');
assert(Buffer.byteLength(socket) < 104, 'private Unix path exceeds native bound');
let processHandle = spawn(process.argv[2], ['--socket', socket], { stdio: ['ignore', 'ignore', 'inherit'] });
let exited = once(processHandle, 'exit');
try {
  let ready = false;
  for (let i = 0; i < 40; i++) {
    try { await lstat(socket); ready = true; break; } catch {}
    if (processHandle.exitCode !== null) throw new Error('real core exited during startup');
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert(ready, 'real core socket unavailable');
  const client = new MeshCoreClient(socket);
  const input = { local_host_id: 'consumer-a', known_hosts: ['peer-b'], hosts: 'peer-b', request_id: 'exact-id', origin_host: 'consumer-a' };
  const first = await client.normalize(input);
  assert.deepEqual(first, { hosts: ['peer-b'], request_id: 'exact-id', origin_host: 'consumer-a', hop_count: 0 });
  await assert.rejects(client.normalize({ local_host_id: 'consumer-c', known_hosts: [], hosts: 'peer-b' }));
  const repeated = await client.normalize(input);
  assert.deepEqual(repeated, first);
  await assert.rejects(client.normalize({ ...input, hosts: '*', hop_count: 1 }));
  processHandle.kill('SIGTERM');
  assert.equal((await exited)[0], 0, 'graceful SIGTERM failed');
  await assert.rejects(lstat(socket), { code: 'ENOENT' });
  processHandle = spawn(process.argv[2], ['--socket', socket], { stdio: ['ignore', 'ignore', 'inherit'] });
  exited = once(processHandle, 'exit');
  for (let i = 0; i < 40; i++) {
    try { await lstat(socket); break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.deepEqual(await client.normalize(input), first, 'same socket restart must preserve stateless route');
  console.log(JSON.stringify({ restartSameSocket: true, realExecutable: true, nodeBinding: true, scopeStateless: true, sameId: true, mutationOrFleetAcceptance: false }));
} finally {
  if (processHandle.exitCode === null) processHandle.kill('SIGTERM');
  const timer = setTimeout(() => processHandle.kill('SIGKILL'), 2000);
  await exited; clearTimeout(timer);
  // Only this exact socket in the owned temporary directory, after child exit.
  await rm(socket, { force: true }); await rmdir(directory);
}
