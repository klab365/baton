import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const sourcePath = new URL('../src/runtime/pi_report_outcome.js', import.meta.url);
const source = (await readFile(sourcePath, 'utf8')).replace(
  'import { Type } from "typebox";',
  'const Type = { String: () => ({}), Optional: value => value, Object: value => value };',
);
const moduleUrl = `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
const { default: registerExtension } = await import(moduleUrl);

async function fixture(t) {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pi-outcome-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  process.env.BATON_PI_ALLOWED_OUTCOMES = '["completed","blocked"]';
  process.env.BATON_PI_OUTCOME_FILE = path.join(dir, 'outcome.json');
  let tool;
  registerExtension({ registerTool: value => { tool = value; } });
  return { tool, output: process.env.BATON_PI_OUTCOME_FILE };
}
const call = (tool, params) => tool.execute('call', params);

test('blocked outcome requires reason and human action and persists both in handoff', async t => {
  const { tool, output } = await fixture(t);
  await assert.rejects(call(tool, { outcome: 'blocked', summary: 'waiting' }), /blocker reason/);
  await assert.rejects(call(tool, { outcome: 'blocked', summary: 'waiting', blocker_reason: 'missing access' }), /blocker reason/);
  await call(tool, { outcome: 'blocked', summary: 'waiting', blocker_reason: ' missing access ', human_action: ' grant access ' });
  assert.deepEqual(JSON.parse(await readFile(output, 'utf8')), {
    outcome: 'blocked', summary: 'waiting\n\nBlocker: missing access\nHuman action needed: grant access',
    blocker_reason: ' missing access ', human_action: ' grant access ',
  });
  await assert.rejects(call(tool, { outcome: 'completed', summary: 'again' }), /already reported/);
});

test('summary rejects empty, disallowed and over-limit payloads, counting UTF-8 bytes', async t => {
  const { tool } = await fixture(t);
  await assert.rejects(call(tool, { outcome: 'completed', summary: '  ' }), /empty/);
  await assert.rejects(call(tool, { outcome: 'nope', summary: 'valid' }), /not allowed/);
  await assert.rejects(call(tool, { outcome: 'blocked', summary: 'x'.repeat(16300), blocker_reason: 'r'.repeat(100), human_action: 'a' }), /16384 bytes/);
  await call(tool, { outcome: 'completed', summary: 'é'.repeat(8192) });
});
