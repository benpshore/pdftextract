// Test-only instrumentation of #252's exact reviewed source. Split the existing
// acquisition boundary without changing its original drop/reopen behavior, then
// copy the deterministic fixture from the correction. Never used by production.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const [corrected, original] = process.argv.slice(2);
assert(corrected && original, 'expected corrected and original checkouts');
const relative = 'crates/tpe-identify/src/source.rs';
const fixed = fs.readFileSync(path.join(corrected, relative), 'utf8');
const destination = path.join(original, relative);
let source = fs.readFileSync(destination, 'utf8');
const boundary = `fn load_pdf(path: &Path, options: &LoadOptions) -> Result<Loaded, LoadError> {
    let snapshot = acquire::snapshot(path, options.max_bytes)?;`;
assert.equal(source.split(boundary).length, 2, 'original acquisition boundary drifted');
assert(source.includes('drop(snapshot);') && source.includes('pipeline::run_job(&job)'),
  'baseline no longer has the reviewed reopen behavior');
source = source.replace(boundary, `${boundary}
    load_pdf_snapshot(path, options, snapshot)
}

fn load_pdf_snapshot(path: &Path, options: &LoadOptions, snapshot: acquire::Snapshot) -> Result<Loaded, LoadError> {`);
const start = fixed.indexOf('    fn text_pdf(');
const end = fixed.indexOf('    #[test]\n    fn collect_walks_', start);
assert(start > 0 && end > start, 'deterministic fixture markers missing');
const marker = '    use tempfile::tempdir;';
assert.equal(source.split(marker).length, 2);
source = source.replace(marker, `${marker}\n\n${fixed.slice(start, end)}`);
fs.writeFileSync(destination, source);
fs.copyFileSync(path.join(corrected, 'crates/tpe-identify/tests/preservation.rs'),
  path.join(original, 'crates/tpe-identify/tests/preservation.rs'));
console.log('Added regression fixtures and a test-only acquisition seam; original extraction behavior retained.');
