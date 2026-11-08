export interface GraphNode { id:string; kind:string; file?:string; start?:number; end?:number; type?:string }
export interface Graph { schemaVersion:1; valid:boolean; nodes:GraphNode[]; edges:{from:string;to:string;kind:string}[] }
export function parseGraph(text:string):Graph {
 const g=JSON.parse(text); const str=(x:unknown)=>typeof x==='string';
 if(g.schemaVersion!==1 || typeof g.valid!=='boolean' || !Array.isArray(g.nodes)||!Array.isArray(g.edges)) throw Error('Unsupported semantic graph schema (expected v1).');
 const ids=new Set<string>();
 for(const n of g.nodes){if(!str(n.id)||!str(n.kind)||ids.has(n.id))throw Error('Invalid graph node.');ids.add(n.id);if(n.file!==undefined&&!str(n.file))throw Error('Invalid source file.');if(n.start!==undefined&&(!Number.isSafeInteger(n.start)||n.start<0||!Number.isSafeInteger(n.end)||n.end<n.start))throw Error('Invalid source span.');}
 for(const e of g.edges)if(!str(e.kind)||!ids.has(e.from)||!ids.has(e.to))throw Error('Invalid graph relationship.');
 return g;
}
export function utf16Offset(text:string,byte:number):number { let bytes=0,offset=0; for(const c of text){if(bytes+Buffer.byteLength(c)>byte)break; bytes+=Buffer.byteLength(c);offset+=c.length;}return offset; }
export function diagnostics(text:string){return [...text.matchAll(/^(.+):bytes (\d+)\.\.(\d+): ([\w_]+): (.*)$/gm)].map(m=>({file:m[1],start:Number(m[2]),end:Number(m[3]),code:m[4],message:m[5]}));}
export function category(n:GraphNode){return n.kind==='module'?'module':n.kind==='definition'?(n.type?.startsWith('Function(')?'function':n.type?.startsWith('Scope(')?'object':'definition'):n.kind;}
