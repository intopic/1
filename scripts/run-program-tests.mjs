// Preserve test failures through Anchor and shell pipelines. Never trust a stale report.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export function validateProgramResults(report) {
  const suites = ['solana-zk-verifier', 'business admission and independent access grants', 'gated-pool (integrimi i një DEX-i)'];
  if (!report?.stats || report.stats.failures !== 0 || report.stats.tests <= 0 ||
      report.stats.pending !== 0 || report.failures?.length !== 0 ||
      report.passes?.length !== report.stats.tests || report.stats.passes !== report.stats.tests) {
    throw Error('Localnet test report is missing, empty, incomplete, pending or failing');
  }
  for (const suite of suites) {
    if (!report.passes.some(test => test.fullTitle.startsWith(suite + ' '))) {
      throw Error(`Required test suite did not pass: ${suite}`);
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const out = resolve(root, 'artifacts/localnet-results.json');
  mkdirSync(dirname(out), { recursive: true });
  rmSync(out, { force: true });
  const run = spawnSync(process.execPath, [resolve(root, 'node_modules/ts-mocha/bin/ts-mocha'),
    '-p', resolve(root, 'tsconfig.json'), '-t', '1000000', '--reporter', 'json',
    '--reporter-option', `output=${out}`,
    'programs/solana-zk-verifier/tests/**/*.ts', 'programs/gated-pool/tests/**/*.ts'],
    { cwd: root, stdio: 'inherit' });
  try {
    if (run.error) throw run.error;
    if (!existsSync(out)) throw Error('Mocha did not produce a test report');
    const report = JSON.parse(readFileSync(out, 'utf8'));
    for (const test of report.passes ?? []) console.log(`PASS ${test.fullTitle}`);
    for (const test of report.failures ?? []) console.error(`FAIL ${test.fullTitle}\n${test.err?.stack ?? ''}`);
    validateProgramResults(report);
    if (run.status !== 0 || run.signal) throw Error(`Mocha exited with ${run.status ?? run.signal}`);
    console.log(`${report.stats.passes} passing, 0 failing, 0 pending (validated JSON report)`);
  } catch (error) {
    console.error(error);
    process.exitCode = 1;
  }
}
