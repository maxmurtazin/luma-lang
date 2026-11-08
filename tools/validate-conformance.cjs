const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const root = path.resolve(__dirname, '..');
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'conformance/manifest.json')));
assert.equal(manifest.execution, 'not-executed');
assert(manifest.cases.length >= 25 && manifest.cases.length <= 30);
const ids = new Set(), categories = new Set(), counts = {};
const matrix = fs.readFileSync(path.join(root, 'site/content/docs/conformance.md'), 'utf8');
for (const c of manifest.cases) {
  assert(/^[a-z][a-z0-9-]+$/.test(c.id)); assert(!ids.has(c.id)); ids.add(c.id);
  categories.add(c.category); assert.equal(c.status, 'specified-not-executed');
  assert.equal(c.entry, 'main.luma'); assert(c.files.includes(c.entry));
  assert.equal(new Set(c.files).size,c.files.length);
  const dir = path.join(root, 'conformance/cases', c.id);
  assert.deepEqual(fs.readdirSync(dir).sort(), [...c.files].sort());
  for (const f of c.files) {
    assert(/^[A-Za-z_][A-Za-z0-9_]*\.luma$/.test(f));
    assert(fs.readFileSync(path.join(dir, f), 'utf8').trim().length > 0);
  }
  const e = c.expected;
  assert(['accept','static-error','runtime-error'].includes(e.outcome));
  counts[e.outcome] = (counts[e.outcome] || 0) + 1;
  if(e.outcome === 'accept') { assert(Object.keys(e.bindings).length); assert(!e.diagnostic); }
  else { assert(/^E_[A-Z_]+$/.test(e.diagnostic)); assert(c.files.includes(e.location.file)); assert.equal(e.location.required,true); }
  assert(c.rationale.length > 15); assert(matrix.includes('| '+c.id+' | '+c.category+' | '+e.outcome+' | Not executed |'));
}
assert.deepEqual(fs.readdirSync(path.join(root,'conformance/cases')).sort(),[...ids].sort());
for(const category of ['lexical','parser','scope','type-inference','module-imports','match','pipelines','objects','arrays','parallel-effects','errors']) assert(categories.has(category));
assert(counts.accept && counts['static-error'] && counts['runtime-error']);
console.log('PASS fixture integrity: '+ids.size+' cases '+JSON.stringify(counts));
console.log('Inventory validation only; run cargo test for syntax and semantic conformance. No interpreter executions.');

// Frontend expectations cover every file while preserving future semantic outcomes.
const frontend = fs.readFileSync(path.join(root, 'conformance/frontend.tsv'), 'utf8').trim().split(/\r?\n/).map(l => l.split('\t'));
const expectedFrontend = manifest.cases.flatMap(c => c.files.map(f => [c.id, f, ['lex-escape','parse-end','parse-braces'].includes(c.id) ? c.expected.diagnostic : 'accept']));
assert.deepEqual(frontend, expectedFrontend);
console.log('PASS frontend expectation coverage; runtime contracts remain unexecuted.');
