const fs=require('fs'),path=require('path'),assert=require('assert/strict');
const en=fs.readdirSync('site/content/en/docs').sort(),ru=fs.readdirSync('site/content/ru/docs').sort();assert.deepEqual(en,ru,'translation page parity');
const version=JSON.parse(fs.readFileSync('editors/vscode/package.json')).version;
for(const f of ['Cargo.toml','Cargo.lock','packaging/windows/luma.iss','packaging/windows/Install-Extension.ps1','packaging/windows/Smoke-Test.ps1'])assert(fs.readFileSync(f,'utf8').includes(version),f+' version');
for(const lang of ['en','ru'])for(const file of en){const slug=file==='_index.md'?'':file.slice(0,-3)+'/';const html=fs.readFileSync('site/public/'+lang+'/docs/'+slug+'index.html','utf8');assert(html.includes('/luma-lang/'+(lang==='en'?'ru':'en')+'/docs/'+slug),'translation switch '+file);}
console.log('PASS translation parity, generated routes/switches and versions');
for(const file of en){const s=fs.readFileSync('site/content/en/docs/'+file,'utf8'),m=s.match(/^aliases: (.*)$/m);if(m)for(const alias of JSON.parse(m[1]))assert(fs.existsSync('site/public'+alias+'index.html'),'legacy route '+alias)}

for(const lang of ['en','ru'])for(const slug of ['',...en.map(f=>'docs/'+(f==='_index.md'?'':f.slice(0,-3)+'/'))]){
 const html=fs.readFileSync('site/public/'+lang+'/'+slug+'index.html','utf8');
 const nav=html.match(/<nav[\s\S]*?<\/nav>/)[0];
 assert(nav.includes('luma-language'),'header language button '+lang+'/'+slug);
 assert(nav.includes('luma-download')&&nav.includes('latest/download/Luma-windows-x64-setup.exe'),'header download '+lang+'/'+slug);
}
console.log('PASS visible header language and download links on all translated pages');
