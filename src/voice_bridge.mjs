// Node bridge to Yapin's official SDK. The SDK owns LiveKit E2EE/key handoff.
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { resolve, dirname } from 'node:path';
import { spawn } from 'node:child_process';
let input = '';
for await (const chunk of process.stdin) { input += chunk; if (input.length > 65536) throw new Error('configuration too large'); }
const config = JSON.parse(input);
let bot, call, ff;
let cleaning;
async function cleanup() {
  if (cleaning) return cleaning;
  cleaning = (async () => {
    ff?.kill('SIGTERM');
    try { await call?.leave(); } finally { if (bot) await bot.leaveVoice(config.server); }
  })();
  return cleaning;
}
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => {
  cleanup().catch(() => {}).finally(() => process.exit(130));
});
try {
  const require = createRequire(resolve(dirname(config.node_modules), 'package.json'));
  const rtc = await import(pathToFileURL(require.resolve('@livekit/rtc-node')).href);
  const { Bot } = await import(pathToFileURL(config.sdk).href);
  bot = new Bot(config.token, { base: config.base_url, log: false, gateway: false });
  call = await bot.connectVoice(rtc, config.server, config.channel);
  const channels = 2, sampleRate = 48000, samples = 480, frameBytes = samples * channels * 2;
  ff = spawn(config.ffmpeg, ['-nostdin', '-loglevel', 'error', '-i', config.file, '-f', 's16le', '-ar', String(sampleRate), '-ac', String(channels), 'pipe:1'], { stdio: ['ignore', 'pipe', 'pipe'] });
  let detail = '';
  ff.stderr.on('data', chunk => { detail = (detail + chunk).slice(-4096); });
  const finished = new Promise(resolve => {
    ff.once('error', error => resolve({ error }));
    ff.once('close', code => resolve({ code }));
  });
  let buffer = Buffer.alloc(0);
  async function capture(bytes) {
    // A copied ArrayBuffer avoids Buffer byte-offset/alignment problems.
    const pcm = new Int16Array(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length));
    await call.source.captureFrame(new rtc.AudioFrame(pcm, sampleRate, channels, samples));
  }
  for await (const chunk of ff.stdout) {
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length >= frameBytes) { await capture(buffer.subarray(0, frameBytes)); buffer = buffer.subarray(frameBytes); }
  }
  const result = await finished;
  if (result.error) throw new Error(`ffmpeg could not start: ${result.error.message}`);
  if (result.code !== 0) throw new Error(`ffmpeg failed (${result.code}): ${detail}`);
  if (buffer.length) { const padded = Buffer.alloc(frameBytes); buffer.copy(padded); await capture(padded); }
  await call.source.waitForPlayout();
} catch (error) {
  console.error(String(error.message || error).replaceAll(config.token, '[REDACTED]'));
  process.exitCode = 1;
} finally {
  try { await cleanup(); } catch { console.error('Voice cleanup failed; use leave_voice to disconnect'); process.exitCode = 1; }
}
