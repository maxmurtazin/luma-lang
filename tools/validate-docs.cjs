const fs=require('fs'),path=require('path'),assert=require('assert/strict');
const en=fs.readdirSync('site/content/en/docs').sort(),ru=fs.readdirSync('site/content/ru/docs').sort();assert.deepEqual(en,ru,'translation page parity');
const version=JSON.parse(fs.readFileSync('editors/vscode/package.json')).version;
for(const f of ['Cargo.toml','Cargo.lock','packaging/windows/luma.iss','packaging/windows/Install-Extension.ps1','packaging/windows/Smoke-Test.ps1'])assert(fs.readFileSync(f,'utf8').includes(version),f+' version');
for(const lang of ['en','ru'])for(const file of en){const slug=file==='_index.md'?'':file.slice(0,-3)+'/';const html=fs.readFileSync('site/public/'+lang+'/docs/'+slug+'index.html','utf8');assert(html.includes('/luma-lang/'+(lang==='en'?'ru':'en')+'/docs/'+slug),'translation switch '+file);}
console.log('PASS translation parity, generated routes/switches and versions');
