const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),assert=require('node:assert/strict'),{spawnSync}=require('node:child_process');
const cli=process.env.LUMA_BIN||path.resolve('target/debug/luma'+(process.platform==='win32'?'.exe':''));
const tmp=fs.mkdtempSync(path.join(os.tmpdir(),'luma-docs-'));let count=0;
try {
for(const lang of ['en','ru'])for(const name of fs.readdirSync('site/content/'+lang+'/docs')){
 if(name==='syntax.md')continue;
 const s=fs.readFileSync('site/content/'+lang+'/docs/'+name,'utf8');
 const blocks=[...s.matchAll(/~~~text\r?\n([\s\S]*?)~~~/g)];
 for(let i=0;i<blocks.length;i+=2){
  assert(blocks[i+1],name+' missing expected result');
  const between=s.slice(blocks[i].index+blocks[i][0].length,blocks[i+1].index);
  assert(/Expected output|Expected diagnostic|Ожидаемый вывод|Ожидаемая диагностика/.test(between),name+' missing label');
  const file=path.join(tmp,'main.luma');fs.writeFileSync(file,blocks[i][1]);
  const r=spawnSync(cli,['run',file],{encoding:'utf8',timeout:30000});if(r.error)throw r.error;
  const expected=blocks[i+1][1].replace(/\r\n/g,'\n');
  if(/Expected diagnostic|Ожидаемая диагностика/.test(between)){assert.notEqual(r.status,0);assert(r.stderr.includes(expected.trim()));}
  else {assert.equal(r.status,0,name+': '+r.stderr);assert.equal(r.stdout.replace(/\r\n/g,'\n'),expected,name);}
  count++;
 }
}
console.log('PASS '+count+' bilingual documentation examples with exact CLI output');
} finally {fs.rmSync(tmp,{recursive:true,force:true});}
