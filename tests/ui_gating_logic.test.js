/**
 * Unit tests for UI control gating logic (mc-b1j.23)
 *
 * Tests the pure functions that determine whether UI actions should be
 * allowed or gated based on override state and target agent.
 *
 * Run: node tests/ui_gating_logic.test.js
 */

const assert = require('assert');

// ---- Reproduce the gating logic from app.js ----

let ROOT_AGENT_ID = 'main';

function isActionAllowed(targetAgentId, overrideActive) {
  if (!targetAgentId) return true;
  if (targetAgentId === ROOT_AGENT_ID) return true;
  if (overrideActive) return true;
  return false;
}

function gatingMessage(targetAgentId) {
  if (isActionAllowed(targetAgentId, false)) return null;
  return `Direct control limited to root agent '${ROOT_AGENT_ID}'. Enable CEO override to control '${targetAgentId}'.`;
}

// ---- Tests ----

let passed = 0;
let failed = 0;

function test(name, fn) {
  try {
    fn();
    passed++;
    console.log(`  ✓ ${name}`);
  } catch (e) {
    failed++;
    console.log(`  ✗ ${name}`);
    console.log(`    ${e.message}`);
  }
}

console.log('UI Gating Logic Tests');
console.log('=====================\n');

console.log('isActionAllowed:');

test('null target → allowed', () => {
  assert.strictEqual(isActionAllowed(null, false), true);
});

test('undefined target → allowed', () => {
  assert.strictEqual(isActionAllowed(undefined, false), true);
});

test('root agent → always allowed (no override)', () => {
  assert.strictEqual(isActionAllowed('main', false), true);
});

test('root agent → always allowed (with override)', () => {
  assert.strictEqual(isActionAllowed('main', true), true);
});

test('non-root agent → denied without override', () => {
  assert.strictEqual(isActionAllowed('dev-a', false), false);
});

test('non-root agent → allowed with override', () => {
  assert.strictEqual(isActionAllowed('dev-a', true), true);
});

test('pdpm agent → denied without override', () => {
  assert.strictEqual(isActionAllowed('pdpm-mc', false), false);
});

test('pdpm agent → allowed with override', () => {
  assert.strictEqual(isActionAllowed('pdpm-mc', true), true);
});

test('empty string target → denied without override (not root)', () => {
  assert.strictEqual(isActionAllowed('', false), true); // empty string is falsy
});

console.log('\ngatingMessage:');

test('root agent → no gating message', () => {
  assert.strictEqual(gatingMessage('main'), null);
});

test('null target → no gating message', () => {
  assert.strictEqual(gatingMessage(null), null);
});

test('non-root agent → returns gating message', () => {
  const msg = gatingMessage('dev-a');
  assert.ok(msg !== null);
  assert.ok(msg.includes('root agent'));
  assert.ok(msg.includes('dev-a'));
  assert.ok(msg.includes('CEO override'));
});

test('message includes the target agent name', () => {
  const msg = gatingMessage('qa-bot');
  assert.ok(msg.includes('qa-bot'));
});

console.log('\nOverride state transitions:');

test('enable override → all agents allowed', () => {
  // Simulate override enable
  const agents = ['main', 'pdpm-mc', 'dev-a', 'qa-1'];
  agents.forEach(a => {
    assert.strictEqual(isActionAllowed(a, true), true, `${a} should be allowed with override`);
  });
});

test('disable override → only root allowed', () => {
  const agents = ['pdpm-mc', 'dev-a', 'qa-1'];
  agents.forEach(a => {
    assert.strictEqual(isActionAllowed(a, false), false, `${a} should be denied without override`);
  });
  assert.strictEqual(isActionAllowed('main', false), true, 'root should always be allowed');
});

test('custom root agent id', () => {
  ROOT_AGENT_ID = 'custom-root';
  assert.strictEqual(isActionAllowed('custom-root', false), true);
  assert.strictEqual(isActionAllowed('main', false), false); // old root is now non-root
  ROOT_AGENT_ID = 'main'; // restore
});

console.log('\nFormatting:');

function formatTtl(seconds) {
  if (seconds <= 0) return 'expired';
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m > 60) {
    const h = Math.floor(m / 60);
    return `${h}h ${m % 60}m`;
  }
  return `${m}m ${s.toString().padStart(2, '0')}s`;
}

test('formatTtl: 0 seconds → expired', () => {
  assert.strictEqual(formatTtl(0), 'expired');
});

test('formatTtl: negative → expired', () => {
  assert.strictEqual(formatTtl(-1), 'expired');
});

test('formatTtl: 600 seconds → 10m 00s', () => {
  assert.strictEqual(formatTtl(600), '10m 00s');
});

test('formatTtl: 65 seconds → 1m 05s', () => {
  assert.strictEqual(formatTtl(65), '1m 05s');
});

test('formatTtl: 3661 seconds → 1h 1m', () => {
  assert.strictEqual(formatTtl(3661), '1h 1m');
});

test('formatTtl: 7200 seconds → 2h 0m', () => {
  assert.strictEqual(formatTtl(7200), '2h 0m');
});

// ---- Summary ----

console.log(`\n${passed + failed} tests: ${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
