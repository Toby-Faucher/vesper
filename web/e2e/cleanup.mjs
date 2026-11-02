// lib.mjs stops what it started when a script dies (#61). For each way to die, a child script starts the server and a
// browser, then crashes; once it has exited, every process it had spawned must be gone and its HTTP port must bind again.
// Run: node e2e/cleanup.mjs (needs the release server and dist/, like the other e2e scripts). About 15 s.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createServer } from 'node:net';

const lib = new URL('./lib.mjs', import.meta.url).href;
const child = (crash) => `
  import { ORIGIN, browser, startServer } from ${JSON.stringify(lib)};
  await startServer();
  await browser('crash');
  console.log('ready ' + ORIGIN);
  await new Promise((r) => process.stdin.once('data', r)); // crash when the parent has listed the processes
  ${crash}`;
const cases = {
  uncaughtException: `throw new Error('boom');`,
  unhandledRejection: `Promise.reject(new Error('boom')); await new Promise(() => {});`,
  SIGTERM: `await new Promise(() => {});`,
  SIGINT: `await new Promise(() => {});`,
};

// Every descendant of `pid`, from ps.
function descendants(pid) {
  const rows = execFileSync('ps', ['-eo', 'pid=,ppid=']).toString().trim().split('\n').map((l) => l.trim().split(/\s+/).map(Number));
  const out = [];
  for (let todo = [pid]; todo.length;) {
    const p = todo.pop();
    for (const [c, pp] of rows) if (pp === p) { out.push(c); todo.push(c); }
  }
  return out;
}
// `pid` with its state and command line while it runs, else null. A zombie (state Z) has exited and waits to be reaped;
// ps exits 1 when no such pid is left, which is the gone case, not an error.
function running(pid) {
  try {
    const row = execFileSync('ps', ['-o', 'pid=,stat=,args=', '-p', String(pid)], { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
    return row && row.split(/\s+/)[1][0] !== 'Z' ? row : null;
  } catch (e) {
    if (e.status === 1) return null;
    throw e;
  }
}
// The processes of `pids` still running after up to 5 s: a child killed by its group can take a moment to go.
async function leftRunning(pids) {
  for (const end = Date.now() + 5000; ; await new Promise((r) => setTimeout(r, 100))) {
    const left = pids.map(running).filter(Boolean);
    if (!left.length || Date.now() > end) return left;
  }
}
const binds = (port) => new Promise((r) => { const s = createServer().once('error', () => r(false)).listen(port, '127.0.0.1', () => s.close(() => r(true))); });

for (const [name, crash] of Object.entries(cases)) {
  const p = spawn(process.execPath, ['--input-type=module', '-e', child(crash)], { stdio: ['pipe', 'pipe', 'pipe'] });
  let err = '';
  p.stderr.on('data', (d) => (err += d));
  const exited = new Promise((r) => p.once('exit', (code, sig) => r(code ?? sig)));
  const origin = await new Promise((r, j) => {
    let out = '';
    p.stdout.on('data', (d) => { out += d; const m = out.match(/ready (\S+)/); if (m) r(m[1]); });
    void exited.then(() => j(new Error(`${name}: the child exited before it was ready\n${err}`)));
  });
  const spawned = descendants(p.pid);
  assert.ok(spawned.length >= 2, `${name}: expected a server and a browser under the child, found ${spawned.length} processes`);
  if (name.startsWith('SIG')) p.kill(name);
  else p.stdin.write('go\n');
  const code = await exited;
  assert.notEqual(code, 0, `${name}: the child exited 0`);
  if (name !== 'SIGTERM' && name !== 'SIGINT') assert.match(err, /Error: boom\n\s+at /, `${name}: no stack printed`);
  const left = await leftRunning(spawned);
  assert.deepEqual(left, [], `${name}: processes left running: ${left.join('; ')}`);
  assert.ok(await binds(Number(new URL(origin).port)), `${name}: port ${new URL(origin).port} still in use`);
  console.log(`cleanup after ${name}: ok (exit ${code}, ${spawned.length} processes stopped)`);
}
