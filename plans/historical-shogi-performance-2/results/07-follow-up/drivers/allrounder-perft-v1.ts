import assert from 'node:assert/strict';
import {Shogi} from '/home/na2hiro/proj/shogitter-3/packages/core/src/index.ts';
import {PossibilityScoringShogiGame} from '/home/na2hiro/proj/shogitter-3/packages/ai/src/players/possibilityistPlayer.ts';
import {perft} from '/home/na2hiro/proj/shogitter-3/packages/ai/src/search/perft.ts';
import {readFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
const [idText,depthText,nativeBinary,nativeName]=process.argv.slice(2);
const ruleId=Number(idText),depth=Number(depthText);
assert.equal(readFileSync('/proc/self/status','utf8').match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1],'7');
const s=Shogi.ofRuleId(ruleId);s.start();const initial=JSON.stringify(s.serialize());
function run(){return perft(new PossibilityScoringShogiGame(s),depth);}
const before=performance.now();const expected=run();let warmupRepetitions=1;
while(performance.now()-before<3000){assert.equal(run(),expected);warmupRepetitions++;}
assert.equal(JSON.stringify(s.serialize()),initial);
const warmupSeconds=(performance.now()-before)/1000;
function sample(){
 const start=performance.now();let repetitions=0,nodes=0;
 do{assert.equal(run(),expected);repetitions++;nodes+=expected;}while(performance.now()-start<3000);
 const seconds=(performance.now()-start)/1000;
 assert.equal(JSON.stringify(s.serialize()),initial);
 return {repetitions,nodesPerPerft:expected,totalLeafNodes:nodes,seconds,nps:nodes/seconds};
}
function nativeSample(){
 const row=JSON.parse(execFileSync(nativeBinary,[nativeName,String(depth),'3000','1'],{encoding:'utf8',timeout:900000}));
 return row.result.samples[0];
}
const allrounder=[],haitaka=[];
for(let i=0;i<3;i++){
 if(i%2===0){allrounder.push(sample());haitaka.push(nativeSample());}
 else{haitaka.push(nativeSample());allrounder.push(sample());}
}
function summary(samples:any[]){
 const sorted=samples.map(s=>s.nps).sort((a,b)=>a-b);const spread=sorted.at(-1)!/sorted[0]-1;
 assert(samples.every(s=>s.seconds>=3));
 return {samples,medianNps:sorted[1],nodesPerPerft:samples[0].nodesPerPerft,sampleRangePercent:spread*100,needsRemeasurement:spread>.05};
}
const h=summary(haitaka),a=summary(allrounder);
console.log(JSON.stringify({ruleId,depth,haitaka:h,allrounder:a,sameLeafCount:h.nodesPerPerft===a.nodesPerPerft,allrounderWarmup:{repetitions:warmupRepetitions,seconds:warmupSeconds}}));
