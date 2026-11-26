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
  # Refresh the engine whenever mbp's build differs from the copy on mba.
  local want=$(ssh mbp "shasum -a 256 proj/hv-donors/target-$R/release/haitaka_cli" | cut -c1-64)
  [ -n "$want" ] || { log "$R no cli on mbp"; return 2; }
  if [ "$(ssh mba "shasum -a 256 $M/haitaka_cli 2>/dev/null" | cut -c1-64)" != "$want" ]; then
    scp -3 -q mbp:proj/hv-donors/target-$R/release/haitaka_cli mba:$M/haitaka_cli || { log "$R copy cli failed"; return 2; }
    log "$R cli updated to ${want:0:12}"
  fi
  for m in $A $B; do [ $m = hc ] && continue
    scp -3 -q mbp:smallnet-rules/$R/models/$m.nnue mba:$M/models/$m.nnue || { log "$R copy $m failed"; return 2; }
  done
  local bside="--b-eval handcrafted"; [ $B != hc ] && bside="--b-eval nnue --b-nnue models/$B.nnue"
  log "$R $N start"
  local pid=$(ssh mba "cd $M && rm -rf matches/$N && nohup caffeinate -i ./haitaka_cli self-play --games ${GAMES:-512} --threads 4 \
     --a-eval nnue --a-nnue models/$A.nnue $bside --movetime-ms 100 --opening-random-plies 6 --seed 23 --max-plies 256 \
     --report-dir matches/$N > matches/$N.log 2>&1 < /dev/null & echo \$!")
  [ -n "$pid" ] || { log "$R $N did not start"; return 2; }
  # Poll until the match process is gone; an unreachable mba (asleep) just waits.
  while true; do
    local st=$(ssh -o ConnectTimeout=20 mba "kill -0 $pid 2>/dev/null && echo run || echo end")
    [ "$st" = end ] && break
    sleep 120
  done
  mkdir -p $D/$R
  scp -q -r mba:$M/matches/$N mba:$M/matches/$N.log $D/$R/ || { log "$R $N fetch failed"; return 2; }
  [ -s $D/$R/$N/self-play-report.json ] || { log "$R $N no report"; tail -3 $D/$R/$N.log >> $D/run.log; return 1; }
  ssh mbp "mkdir -p smallnet-rules/$R/matches" && scp -q -r $D/$R/$N $D/$R/$N.log mbp:smallnet-rules/$R/matches/
  ssh mba "rm -rf $M/matches/$N"
  summary $D/$R/$N/self-play-report.json $R $N | tee -a $D/results.txt | ssh mbp "cat >> smallnet-rules/$R/results.txt"
  log "$R $N done"
}
log start
while true; do
  did=0
  if ! ssh -o ConnectTimeout=20 mba true 2>/dev/null; then sleep 600; continue; fi
  for R in $(cat $D/rules.txt); do
    for i in 1 2 3 4; do
      ssh -o ConnectTimeout=20 mbp "test -s smallnet-rules/$R/models/kf$i.nnue" || continue
      for B in hc prev; do
        if [ $B = prev ]; then [ $i = 1 ] && continue; B=kf$((i - 1)); N=kf$i-vs-$B; else N=kf$i-vs-hc; fi
        [ -s $D/$R/$N/self-play-report.json ] && continue
        [ -e $D/$R/$N.failed ] && continue
        # Status 1 = the match itself failed (marked, not retried); 2 = transfer trouble (retried).
        run_match $R $N kf$i $B; rc=$?
        [ $rc = 1 ] && { mkdir -p $D/$R; touch $D/$R/$N.failed; }
        did=1
      done
    done
  done
  [ $did = 0 ] && sleep 900
done
