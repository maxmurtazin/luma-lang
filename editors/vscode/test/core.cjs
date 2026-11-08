const {test}=require('node:test');const a=require('node:assert/strict');const fs=require('node:fs');const c=require('../dist/core');const fixture=fs.readFileSync(__dirname+'/fixtures/schema-v1.json','utf8');
test('schema and declaration categories',()=>{const g=c.parseGraph(fixture);a.equal(c.category(g.nodes[1]),'function');a.equal(c.category({kind:'definition',type:'Scope(1)'}),'object');});
test('reject schema mismatch, duplicates, dangling edges and bad spans',()=>{for(const mutate of [g=>g.schemaVersion=2,g=>g.nodes.push(g.nodes[0]),g=>g.edges[0].to='missing',g=>g.nodes[1].start=-1]){const g=JSON.parse(fixture);mutate(g);a.throws(()=>c.parseGraph(JSON.stringify(g)));}a.throws(()=>c.parseGraph('bad'));});
test('empty partial graph',()=>{a.equal(c.parseGraph(JSON.stringify({schemaVersion:1,valid:false,nodes:[],edges:[]})).nodes.length,0);});
test('UTF8 and UTF16 spans with astral characters and CRLF',()=>{a.equal(c.utf16Offset('é😀\r\nx',6),3);a.equal(c.utf16Offset('é😀\r\nx',8),5);});
test('diagnostic messages and Windows paths',()=>{const file=String.raw`C:workmain.luma`;const d=c.diagnostics(file+':bytes 2..6: E_TYPE: bad: type');a.equal(d[0].file,file);a.equal(d[0].end,6);a.equal(d[0].message,'bad: type');});
if(process.env.LUMA_GRAPH)test('actual CLI schema',()=>a.ok(c.parseGraph(fs.readFileSync(process.env.LUMA_GRAPH,'utf8')).nodes.length));
