const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs/promises');
const os=require('node:os');
const path=require('node:path');
const {resolveExecutable}=require('../dist/executable');
test('Windows installer discovery works with spaces, preserves overrides and falls back when absent',async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'luma install '));
 try {
  assert.equal(await resolveExecutable('luma','win32',root),'luma');
  const bin=path.join(root,'Programs','Luma','bin');
  await fs.mkdir(bin,{recursive:true});
  const exe=path.join(bin,'luma.exe');await fs.writeFile(exe,'fixture');
  assert.equal(await resolveExecutable('luma','win32',root),exe);
  assert.equal(await resolveExecutable('custom.exe','win32',root),'custom.exe');
  assert.equal(await resolveExecutable('luma','linux',root),'luma');
  assert.equal(await resolveExecutable('luma','win32',''),'luma');
 }finally{await fs.rm(root,{recursive:true,force:true});}
});
