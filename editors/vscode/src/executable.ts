import * as fs from 'node:fs/promises';
import * as path from 'node:path';

// Keep explicit settings intact; only the default uses the bundled Windows CLI.
export async function resolveExecutable(configured:string, platform=process.platform, localAppData=process.env.LOCALAPPDATA):Promise<string> {
 if(configured!=='luma'||platform!=='win32'||!localAppData)return configured;
 const installed=path.join(localAppData,'Programs','Luma','bin','luma.exe');
 return await fs.access(installed).then(()=>installed,()=>configured);
}
