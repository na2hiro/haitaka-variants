#!/bin/bash
# Plays small-net evaluation matches on mba as nets appear on mbp (mbp is ML-only).
# For each rule in rules.txt (re-read every pass) and each kf<i> on mbp: kf<i>-vs-hc, and
# kf<i>-vs-kf<i-1> for i>1. 100 ms/move, startpos + 6 random plies, seed 23, 512 games,
# 4 threads (one per mba performance core). Reports go back to mbp <rule>/matches/ and
# here; one summary line per match goes to results.txt.
D=$(cd $(dirname $0) && pwd)
log() { echo "$(date '+%m-%d %H:%M') $*" >> $D/run.log; }
summary() { python3 -c "
import json,sys;s=json.load(open(sys.argv[1]))['summary'];a,b=s['aBreakdown'],s['bBreakdown']
print(sys.argv[2],sys.argv[3],'games',s['games'],'A score %.4f'%s['scoreRate'],'W-L-D',s['aWins'],s['bWins'],s['draws'],
 'pairedElo %.1f'%s['pairedElo'],[round(x,1) for x in s['pairedElo95Ci']],'fallbacks A/B',a['fallbacks'],b['fallbacks'],
 'nps A/B %.0f/%.0f'%(a['aggregateNps'],b['aggregateNps']),'depth A/B',a['completedDepth'],b['completedDepth'])" "$@"; }
run_match() { # rule name modelA modelB-or-hc
  local R=$1 N=$2 A=$3 B=$4 M=smallnet-rules/$1
  ssh mba "mkdir -p $M/models $M/matches"
  if ! ssh mba "test -x $M/haitaka_cli"; then
    scp -3 -q mbp:proj/hv-donors/target-$R/release/haitaka_cli mba:$M/haitaka_cli || { log "$R copy cli failed"; return 1; }
  fi
  for m in $A $B; do [ $m = hc ] && continue
    scp -3 -q mbp:smallnet-rules/$R/models/$m.nnue mba:$M/models/$m.nnue || { log "$R copy $m failed"; return 1; }
  done
  local bside="--b-eval handcrafted"; [ $B != hc ] && bside="--b-eval nnue --b-nnue models/$B.nnue"
  log "$R $N start"
  local pid=$(ssh mba "cd $M && rm -rf matches/$N && nohup caffeinate -i ./haitaka_cli self-play --games ${GAMES:-512} --threads 4 \
     --a-eval nnue --a-nnue models/$A.nnue $bside --movetime-ms 100 --opening-random-plies 6 --seed 23 --max-plies 256 \
     --report-dir matches/$N > matches/$N.log 2>&1 < /dev/null & echo \$!")
  while ssh -o ConnectTimeout=20 mba "kill -0 $pid 2>/dev/null"; do sleep 120; done
  mkdir -p $D/$R
  scp -q -r mba:$M/matches/$N mba:$M/matches/$N.log $D/$R/ || { log "$R $N fetch failed"; return 1; }
  [ -s $D/$R/$N/self-play-report.json ] || { log "$R $N no report"; tail -3 $D/$R/$N.log >> $D/run.log; return 1; }
  ssh mbp "mkdir -p smallnet-rules/$R/matches" && scp -q -r $D/$R/$N $D/$R/$N.log mbp:smallnet-rules/$R/matches/
  ssh mba "rm -rf $M/matches/$N"
  summary $D/$R/$N/self-play-report.json $R $N | tee -a $D/results.txt | ssh mbp "cat >> smallnet-rules/$R/results.txt"
  log "$R $N done"
}
log start
while true; do
  did=0
  for R in $(cat $D/rules.txt); do
    for i in 1 2 3 4; do
      ssh -o ConnectTimeout=20 mbp "test -s smallnet-rules/$R/models/kf$i.nnue" || continue
      for B in hc prev; do
        if [ $B = prev ]; then [ $i = 1 ] && continue; B=kf$((i - 1)); N=kf$i-vs-$B; else N=kf$i-vs-hc; fi
        [ -s $D/$R/$N/self-play-report.json ] && continue
        [ -e $D/$R/$N.failed ] && continue
        run_match $R $N kf$i $B || { mkdir -p $D/$R; touch $D/$R/$N.failed; }
        did=1
      done
    done
  done
  [ $did = 0 ] && sleep 900
done
