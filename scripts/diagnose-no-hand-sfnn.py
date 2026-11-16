#!/usr/bin/env python3
"""Matched interventions for SFNN on discard-capture rules. Native diagnostics only."""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]

def save(p, v): p.write_text(json.dumps(v, ensure_ascii=False, indent=2) + "\n")
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()

def summarize(out):
    rows=[];suites={}
    for d in sorted(out.glob("matches/*")):
        report=d/"self-play-report.json"
        if not report.exists(): continue
        r=json.loads(report.read_text());games=[json.loads(s) for s in (d/"self-play-games.jsonl").read_text().splitlines()]
        pairs=defaultdict(list)
        for g in games:
            assert g["failureState"] is None
            assert g["pairIndex"]==(g["gameIndex"]-1)//2
            assert g["aColor"]==("black" if g["gameIndex"]%2 else "white")
            pairs[g["pairIndex"]].append(g)
        assert len(games)==24 and len(pairs)==12
        for pair in pairs.values(): assert len(pair)==2 and pair[0]["startSfen"]==pair[1]["startSfen"]
        rule=d.name.split("-",1)[0];suite={k:p[0]["startSfen"] for k,p in pairs.items()}
        if rule in suites: assert suites[rule]==suite
        suites[rule]=suite
        c=Counter(g["winner"] for g in games);s=r["summary"]
        assert (c["A"],c["B"],c[None])==(s["aWins"],s["bWins"],s["draws"])
        rows.append({"case":d.name,"games":len(games),"aWins":c["A"],"bWins":c["B"],"draws":c[None],"scoreRate":(c["A"]+.5*c[None])/len(games),"terminations":dict(Counter(g["terminationReason"] for g in games)),"a":s["aBreakdown"],"b":s["bBreakdown"],"gamesSha256":sha(d/"self-play-games.jsonl"),"reportSha256":sha(report)})
    save(out/"match-summary.json",{"rows":rows})
    print(json.dumps(rows,ensure_ascii=False,indent=2),flush=True)

def main():
    p=argparse.ArgumentParser();p.add_argument("phase",choices=["run","summarize"]);p.add_argument("--output",type=Path,default=ROOT/"target/no-hand-diagnosis");p.add_argument("--model",type=Path);a=p.parse_args()
    out=a.output.resolve()
    if a.phase=="summarize":summarize(out);return
    if not a.model:p.error("--model is required to run")
    model=a.model.resolve();assert sha(model/"nn.bin")=="7f9545dbf73b87acf312146daabf59c4f17d0845c87d587fa19caed51707dddb"
    assert sha(model/"progress.bin")=="a2d18f0564639c945b5d6469e9b437cd64a526ad4fbcef02ab1d8f547d08e5f1"
    assert not subprocess.check_output(["git","status","--porcelain","--untracked-files=no"],cwd=ROOT).strip()
    cases=[("standard","nodes-baseline",[])]
    for rule in ["heian","fushiyo"]:
        cases += [(rule,"time-baseline",[]),(rule,"nodes-baseline",[]),(rule,"nodes-ghost",["--ghost-hands"]),(rule,"nodes-zero",["--zero-missing-slot"])]
    conditions={"sourceCommit":subprocess.check_output(["git","rev-parse","HEAD"],cwd=ROOT,text=True).strip(),"createdUtc":datetime.now(timezone.utc).isoformat(),"gamesPerCase":24,"pairsPerCase":12,"maxPlies":512,"seed":330011,"randomOpeningPlies":8,"movetimeMs":100,"nodesPerMove":4096,"threads":4,"cpuAffinity":sorted(os.sched_getaffinity(0)),"modelSha256":sha(model/"nn.bin"),"progressSha256":sha(model/"progress.bin"),"binaries":{f.name:sha(f) for f in sorted((out/"bin").glob("*-engine"))}|{f.name:sha(f) for f in sorted((out/"bin").glob("*-cli"))},"cases":cases}
    assert not (out/"match-conditions.json").exists(),"preserve previous runs and use a fresh directory"
    save(out/"match-conditions.json",conditions);(out/"matches").mkdir(exist_ok=True)
    for rule,name,flags in cases:
        d=out/"matches"/f"{rule}-{name}"
        cmd=[out/"bin"/f"{rule}-cli","self-play","--games",24,"--threads",4,"--max-plies",512,"--opening-random-plies",8,"--seed",330011,"--a-engine",out/"bin"/f"{rule}-engine","--a-engine-arg=--model-dir","--a-engine-arg",model]
        for flag in flags:cmd += [f"--a-engine-arg={flag}"]
        cmd += ["--b-engine",out/"bin"/f"{rule}-engine","--report-dir",d]
        cmd += ["--movetime-ms",100] if name.startswith("time-") else ["--nodes-per-move",4096]
        save(out/"matches"/f"{rule}-{name}-command.json",list(map(str,cmd)))
        print("RUN",rule,name,flush=True);started=time.monotonic()
        with (out/"matches"/f"{rule}-{name}.log").open("w") as stream:
            subprocess.run(list(map(str,cmd)),cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT,check=True)
        print("DONE",rule,name,round(time.monotonic()-started,2),flush=True)
    summarize(out)

if __name__=="__main__":main()
