#!/usr/bin/env node
// Fresh Native JSON-lines oracle versus a previously independently checked corpus.
// Usage: node scripts/verify-native-oracle.mjs target RULE OUTPUT.json
import assert from 'node:assert/strict';
import {createReadStream,existsSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {createInterface} from 'node:readline';
import {createGunzip} from 'node:zlib';
import {spawn} from 'node:child_process';
import {resolve,join} from 'node:path';
const [root,rule,output]=process.argv.slice(2);
assert(root&&/^[a-z0-9-]+$/.test(rule)&&output);
const input=join(resolve(root),rule+'-reference-input.jsonl');
const plain=join(resolve(root),rule+'-reference-output.jsonl');
const golden=existsSync(plain)?plain:plain+'.gz';
const source=createReadStream(golden);
const expected=createInterface({input:golden.endsWith('.gz')?source.pipe(createGunzip()):source,crlfDelay:Infinity})[Symbol.asyncIterator]();
const child=spawn(join(resolve(root),'haitaka-'+rule),['json','oracle'],{stdio:['pipe','pipe','inherit']});
const done=new Promise((resolve,reject)=>{child.on('error',reject);child.on('close',code=>code===0?resolve():reject(Error('Native exit '+code)));});
createReadStream(input).pipe(child.stdin);
const digest=createHash('sha256');let positions=0,transitions=0;
for await(const line of createInterface({input:child.stdout,crlfDelay:Infinity})){
 const reference=await expected.next();
 assert(!reference.done&&line===reference.value,'Native mismatch at position '+positions);
 digest.update(line+'\n');positions++;transitions+=JSON.parse(line).transitions.length;
}
await done;assert((await expected.next()).done);assert(positions>0);
const report={rule,positions,transitions,byteIdentical:true,outputSha256:digest.digest('hex'),reference:golden,native:'Fresh binary; golden corpus reused from previous independent Shogitter comparison.'};
writeFileSync(output,JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
