import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const built = await build({ entryPoints: ['scripts/folder-regression-cases.mjs'], bundle: true, write: false, format: 'esm', platform: 'browser' });
const { runFolderRegressions } = await import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].text).toString('base64')}`);
const outcomes = await runFolderRegressions();
for (const result of outcomes) console.log(result.ok ? 'PASS' : 'FAIL', result.name, result.error || '');
if (outcomes.some(result => !result.ok)) throw new Error('Folder integrity regression failed');
