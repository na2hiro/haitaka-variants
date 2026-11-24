#!/bin/bash
# Small king-free NNUE pipeline for one single-donor rule, following the Anhoku recipe.
# usage: small_net_rule_pipeline.sh <rule> [jobs=8] [threads=8] [generations=4]
# g1 is taught by the handcrafted eval, each later generation by the previous net
# (10k nodes/move). Every net is trained from scratch on all data so far for ~25 epochs
# and exported from its last checkpoint. Stops early when a generation gains < 15 Elo.
set -u
R=$1; J=${2:-8}; T=${3:-8}; GENS=${4:-4}
SRC=~/proj/hv-donors
TGT=$SRC/target-$R
LEARN=$TGT/release/haitaka_learn
CLI=$TGT/release/haitaka_cli
PY=~/smallnet/venv/bin/python
TRAINER=$SRC/scripts/small_net_train.py
W=~/smallnet-rules/$R
mkdir -p $W/gen $W/feat $W/runs $W/models $W/matches
cd $W
log() { echo "$(date '+%m-%d %H:%M') $*"; }

if [ ! -x $LEARN ] || [ ! -x $CLI ]; then
  log build
  (cd $SRC && CARGO_TARGET_DIR=$TGT cargo build -q --release -p haitaka_learn --features $R,nnue-l1-256 \
     && CARGO_TARGET_DIR=$TGT cargo build -q --release -p haitaka_cli --features $R,nnue-l1-256) > build.log 2>&1 \
     || { log "build failed"; tail -20 build.log; exit 1; }
fi

gen() { # name games seed [teacher]
  local teacher=""; [ -n "${4:-}" ] && teacher="--nnue $4"
  if [ ! -s gen/$1.bin ]; then
    caffeinate -i $LEARN small-net-generate --games $2 --nodes ${NODES:-10000} --random-plies 8 --random-move-rate 0.05 \
      --adjudicate-score 3000 --adjudicate-plies 4 --max-plies 200 --seed $3 --jobs $J $teacher \
      --output gen/$1.bin > gen/$1.log 2>&1 || { log "generate $1 failed"; exit 1; }
  fi
  if [ ! -s feat/$1.feat ]; then
    $LEARN small-net-export-features --input gen/$1.bin --output feat/$1.feat > feat/$1-export.log 2>&1 \
      || { log "export $1 failed"; exit 1; }
  fi
  log "$1: $(python3 -c "import json;d=json.load(open('gen/$1.json'));print(d['positions'],'positions',round(d['elapsedSec']),'s')")"
}

rows() { # comma list -> total rows (396 bytes per row)
  local n=0; for f in ${1//,/ }; do n=$((n + $(stat -f %z $f) / 396)); done; echo $n
}

train() { # name trainlist
  if [ ! -s models/$1.nnue ]; then
    local n=$(rows $2)
    local steps=$(( (n * 25 / 16384 + 1023) / 1024 * 1024 )); [ $steps -lt 4096 ] && steps=4096
    log "train $1 on $n rows, $steps steps"
    caffeinate -i $PY $TRAINER train --rule $R --seed 1 --init nnue --optimizer ranger --ranger-dir /Users/na2hiro/smallnet \
      --loss huber --lr 1.5e-3 --lr-floor 0.0333 --output-lr-mult 0.1 --warmup 64 --eval-every 4096 --threads $T \
      --horizon $steps --max-steps $steps --train $2 --val feat/val.feat --out runs/$1 > runs/$1.log 2>&1 \
      || { log "train $1 failed"; tail -5 runs/$1.log; exit 1; }
    $PY $TRAINER export --rule $R --checkpoint runs/$1/last.pt --output models/$1.nnue --description "king-free L1=256 $R $1" > /dev/null
    cp runs/$1/summary.json runs/$1-summary.json
  fi
  log "$1 sha256 $(shasum -a 256 models/$1.nnue | cut -c1-64)"
}

match() { # name A B   (A/B: nnue path or handcrafted); 100ms, startpos + 6 random plies, 512 games
  if [ ! -s matches/$1/self-play-report.json ]; then
    side() { if [ "$2" = handcrafted ]; then echo "--$1-eval handcrafted"; else echo "--$1-eval nnue --$1-nnue $2"; fi; }
    caffeinate -i $CLI self-play --games 512 --threads $((J - 2)) $(side a $2) $(side b $3) --movetime-ms 100 \
      --opening-random-plies 6 --seed 23 --max-plies 256 --report-dir matches/$1 > matches/$1.log 2>&1
  fi
  [ -s matches/$1/self-play-report.json ] || { log "match $1 failed"; tail -5 matches/$1.log; exit 1; }
  python3 -c "
import json;s=json.load(open('matches/$1/self-play-report.json'))['summary']
print('$1','games',s['games'],'A score %.4f'%s['scoreRate'],'W-L-D',s['aWins'],s['bWins'],s['draws'],'pairedElo %.1f'%s['pairedElo'],[round(x,1) for x in s['pairedElo95Ci']],'fallbacks',s['fallbacks'])" | tee -a results.txt
}

elo() { python3 -c "import json;print(int(json.load(open('matches/$1/self-play-report.json'))['summary']['pairedElo']))"; }

log "start $R jobs=$J threads=$T"
gen val 1600 9999
# Size the generations from the measured val speed (1600 games, 10k nodes, handcrafted) so
# that g1 takes about 6.5 h and later generations about 9 h at 8 jobs; rules over 2x that budget drop to
# 5k nodes/move first. GAMES1 / GAMESN / NODES in the environment override this.
read AUTO_NODES AUTO_G1 AUTO_GN < <(python3 -c "
import json; d=json.load(open('gen/val.json')); spg=d['elapsedSec']/d['games']
nodes=10000
if spg*64000 > 46800: nodes=5000; spg/=2
r=lambda b,cap: max(8000, min(cap, int(b/spg)//1600*1600))
print(nodes, r(23400,64000), r(32400,96000))")
NODES=${NODES:-$AUTO_NODES}; GAMES1=${GAMES1:-$AUTO_G1}; GAMESN=${GAMESN:-$AUTO_GN}
log "settings nodes=$NODES games1=$GAMES1 gamesN=$GAMESN"
gen g1 ${GAMES1:-64000} 101
LIST=feat/g1.feat
train kf1 $LIST
match kf1-vs-hc models/kf1.nnue handcrafted
for i in $(seq 2 $GENS); do
  p=$((i - 1))
  gen g$i ${GAMESN:-96000} $((i * 101)) models/kf$p.nnue
  LIST=$LIST,feat/g$i.feat
  train kf$i $LIST
  match kf$i-vs-kf$p models/kf$i.nnue models/kf$p.nnue
  match kf$i-vs-hc models/kf$i.nnue handcrafted
  if [ $(elo kf$i-vs-kf$p) -lt 15 ]; then log "kf$i gained < 15 Elo over kf$p; stopping"; break; fi
done
log ALLDONE
