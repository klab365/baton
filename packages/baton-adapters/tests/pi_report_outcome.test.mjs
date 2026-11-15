import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const sourcePath = new URL('../src/runtime/pi_report_outcome.js', import.meta.url);
const source = (await readFile(sourcePath, 'utf8')).replace(
  'import { Type } from "typebox";',
  'const Type = { String: () => ({}), Number: () => ({}), Any: () => ({}), Optional: value => value, Object: value => value };',
);
const moduleUrl = `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
const { default: registerExtension } = await import(moduleUrl);

async function fixture(t) {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pi-outcome-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  process.env.BATON_PI_ALLOWED_OUTCOMES = '["completed","blocked"]';
  process.env.BATON_PI_OUTCOME_FILE = path.join(dir, 'outcome.json');
  process.env.BATON_PI_HANDOFF_HISTORY_PATH = path.join(dir, 'history.json');
  await writeFile(process.env.BATON_PI_HANDOFF_HISTORY_PATH, JSON.stringify([{attempt: 7, stepId: 'build', report: {version: 1, summary: 'Ready'}}]));
  const tools = {};
  registerExtension({ registerTool: value => { tools[value.name] = value; } });
  return { tool: tools.report_outcome, historyTool: tools.get_structured_handoff, output: process.env.BATON_PI_OUTCOME_FILE };
}
const call = (tool, params) => tool.execute('call', params);

test('blocked outcome needs a reason and preserves optional human action in the handoff', async t => {
  const { tool, output } = await fixture(t);
  await assert.rejects(call(tool, { outcome: 'blocked', summary: 'waiting' }), /blocker reason/);
  await assert.rejects(call(tool, { outcome: 'blocked', summary: 'waiting', blocker_reason: '  ' }), /blocker reason/);
  await call(tool, { outcome: 'blocked', summary: 'waiting', blocker_reason: ' missing access ', human_action: ' grant access ' });
  assert.deepEqual(JSON.parse(await readFile(output, 'utf8')), {
    outcome: 'blocked', summary: 'waiting\n\nBlocker: missing access\nHuman action needed: grant access',
    blocker_reason: ' missing access ', human_action: ' grant access ',
  });
  const omitted = await fixture(t);
  await call(omitted.tool, { outcome: 'blocked', summary: 'waiting', blocker_reason: 'missing access' });
  assert.equal(JSON.parse(await readFile(omitted.output, 'utf8')).summary, 'waiting\n\nBlocker: missing access');
});

test('managed agents can retrieve an omitted report by attempt', async t => {
  const { historyTool } = await fixture(t);
  const result = await historyTool.execute('call', { attempt: 7 });
  assert.match(result.content[0].text, /"stepId":"build"/);
  await assert.rejects(historyTool.execute('call', { attempt: 8 }), /No structured handoff/);
});

test('structured handoff is forwarded without interpreting legacy free text', async t => {
  const { tool, output } = await fixture(t);
  assert.match(tool.description, /Status is open, resolved, or accepted/);
  assert.match(tool.description, /independently verified/);
  const structured_handoff = { version: 1, summary: 'Ready', findings: [] };
  await call(tool, { outcome: 'completed', summary: '{not parsed as structured}', structured_handoff });
  assert.deepEqual(JSON.parse(await readFile(output, 'utf8')).structured_handoff, structured_handoff);
});

test('invalid structured handoffs return actionable errors and allow corrected retry', async t => {
  const { tool, output } = await fixture(t);
  await assert.rejects(call(tool, { outcome: 'completed', summary: 'ready', structured_handoff: { version: 2, summary: 'bad' } }), /version must be 1/);
  await call(tool, { outcome: 'completed', summary: 'ready', structured_handoff: { version: 1, summary: 'ok', findings: [] } });
  assert.equal(JSON.parse(await readFile(output, 'utf8')).structured_handoff.summary, 'ok');
});

test('explicit null structured handoff matches omission', async t => {
  const { tool, output } = await fixture(t);
  await call(tool, { outcome: 'completed', summary: 'ready', structured_handoff: null });
  assert.equal(JSON.parse(await readFile(output, 'utf8')).structured_handoff, null);
});

test('summary rejects empty, disallowed and over-limit payloads, counting UTF-8 bytes', async t => {
  const { tool } = await fixture(t);
  await assert.rejects(call(tool, { outcome: 'completed', summary: '  ' }), /empty/);
  await assert.rejects(call(tool, { outcome: 'nope', summary: 'valid' }), /not allowed/);
  await assert.rejects(call(tool, { outcome: 'blocked', summary: 'x'.repeat(16300), blocker_reason: 'r'.repeat(100), human_action: 'a' }), /16384 bytes/);
  await call(tool, { outcome: 'completed', summary: 'é'.repeat(8192) });
});
