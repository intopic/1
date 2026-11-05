import test from 'node:test';
import assert from 'node:assert/strict';
import { validateProgramResults } from './run-program-tests.mjs';

const valid = () => ({ stats: { tests: 3, passes: 3, failures: 0, pending: 0 }, failures: [],
  passes: ['solana-zk-verifier', 'business admission and independent access grants', 'gated-pool (integrimi i një DEX-i)']
    .map(suite => ({ fullTitle: suite + ' example' })) });

test('accepts a complete passing report', () => { validateProgramResults(valid()); });
test('rejects failures even if the parent process returns success', () => {
  const report = valid(); report.stats.failures = 1; report.failures.push({});
  assert.throws(() => validateProgramResults(report));
});
test('rejects empty, skipped, incomplete or missing-suite reports', () => {
  for (const mutate of [r => { r.stats.tests = 0; }, r => { r.stats.pending = 1; },
    r => { r.passes.pop(); }, r => { r.passes[2].fullTitle = 'unrelated suite test'; }]) {
    const report = valid(); mutate(report);
    assert.throws(() => validateProgramResults(report));
  }
  assert.throws(() => validateProgramResults(null));
});
