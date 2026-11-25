#!/bin/bash
# Small king-free NNUE pipeline for one donor rule: generation and training only.
# (mbp is reserved for ML; evaluation matches run on another host.)
# usage: small_net_rule_pipeline.sh <rule> [jobs=8] [threads=8] [generations=4]
# g1 is taught by the handcrafted eval, each later generation by the previous net. Every
# net is trained from scratch on all data so far for ~25 epochs and exported from its
# last checkpoint (~25 epochs, at least 1024 steps).
#
# Sizing: seconds per game at 10k nodes come from an existing 10k-node val set, or else
# from a 48-game probe at 2k nodes (x5). Nodes per move is the largest of 10k/5k/2.5k/1k
# that still fits 16k games in the 6.5 h g1 budget (1k if none does). g1 then gets
# 6.5 h and later generations 9 h of games (8k..64k / 8k..96k); val gets ~1 h
# (200..1600 games) at the same nodes. GAMES1 / GAMESN / NODES / GAMESVAL override.
set -u
R=$1; J=${2:-8}; T=${3:-8}; GENS=${4:-4}
SRC=~/proj/hv-donors
TGT=$SRC/target-$R
LEARN=$TGT/release/haitaka_learn
PY=~/smallnet/venv/bin/python
TRAINER=$SRC/scripts/small_net_train.py
W=~/smallnet-rules/$R
mkdir -p $W/gen $W/feat $W/runs $W/models
cd $W
log() { echo "$(date '+%m-%d %H:%M') $*"; }

if [ ! -x $LEARN ]; then
  log build
  (cd $SRC && CARGO_TARGET_DIR=$TGT cargo build -q --release -p haitaka_learn --features $R,nnue-l1-256) > build.log 2>&1 \
     || { log "build failed"; tail -20 build.log; exit 1; }
fi

generate() { # name games seed nodes [teacher]
  local teacher=""; [ -n "${5:-}" ] && teacher="--nnue $5"
  if [ ! -s gen/$1.bin ]; then
    caffeinate -i $LEARN small-net-generate --games $2 --nodes $4 --random-plies 8 --random-move-rate 0.05 \
      --adjudicate-score 3000 --adjudicate-plies 4 --max-plies 200 --seed $3 --jobs $J $teacher \
      --output gen/$1.bin > gen/$1.log 2>&1 || { log "generate $1 failed"; tail -3 gen/$1.log; exit 1; }
  fi
}
gen() { # name games seed nodes [teacher]: generate, export features
  generate "$@"
  if [ ! -s feat/$1.feat ]; then
    $LEARN small-net-export-features --input gen/$1.bin --output feat/$1.feat > feat/$1-export.log 2>&1 \
      || { log "export $1 failed"; tail -3 feat/$1-export.log; exit 1; }
  fi
  log "$1: $(python3 -c "import json;d=json.load(open('gen/$1.json'));print(d['positions'],'positions',d['games'],'games',d['nodesPerMove'],'nodes',round(d['elapsedSec']),'s')")"
}

rows() { # comma list -> total rows (row size from the export log)
  local n=0; for f in ${1//,/ }; do
    local b=$(grep -o '([0-9]* bytes each' ${f%.feat}-export.log | grep -o '[0-9]*')
    n=$((n + $(stat -f %z $f) / b)); done; echo $n
}

train() { # name trainlist
  if [ ! -s models/$1.nnue ]; then
    local n=$(rows $2)
    local steps=$(( (n * 25 / 16384 + 1023) / 1024 * 1024 )); [ $steps -lt 1024 ] && steps=1024
    log "train $1 on $n rows, $steps steps"
    caffeinate -i $PY $TRAINER train --rule $R --seed 1 --init nnue --optimizer ranger --ranger-dir /Users/na2hiro/smallnet \
      --loss huber --lr 1.5e-3 --lr-floor 0.0333 --output-lr-mult 0.1 --warmup 64 --eval-every $((steps < 4096 ? steps : 4096)) --threads $T \
      --horizon $steps --max-steps $steps --train $2 --val feat/val.feat --out runs/$1 > runs/$1.log 2>&1 \
      || { log "train $1 failed"; tail -5 runs/$1.log; exit 1; }
    $PY $TRAINER export --rule $R --checkpoint runs/$1/last.pt --output models/$1.nnue --description "king-free L1=256 $R $1" > /dev/null
    cp runs/$1/summary.json runs/$1-summary.json
  fi
  log "$1 sha256 $(shasum -a 256 models/$1.nnue | cut -c1-64)"
}

log "start $R jobs=$J threads=$T"
if [ -s gen/val.json ] && python3 -c "import json,sys;sys.exit(json.load(open('gen/val.json'))['nodesPerMove']!=10000)"; then
  SPEED="gen/val.json 1"
else
  generate probe 48 7777 2000
  SPEED="gen/probe.json 5"
fi
read AUTO_NODES AUTO_VAL AUTO_G1 AUTO_GN < <(python3 -c "
import json; f,k='$SPEED'.split(); d=json.load(open(f)); spg10=d['elapsedSec']/d['games']*int(k)
nodes=next((n for n in (10000,5000,2500,1000) if spg10*n/10000*16000 <= 23400), 1000)
spg=spg10*nodes/10000
r=lambda b,lo,hi,q: max(lo, min(hi, int(b/spg)//q*q))
print(nodes, r(3600,200,1600,100), r(23400,8000,64000,1600), r(32400,8000,96000,1600))")
NODES=${NODES:-$AUTO_NODES}; GAMESVAL=${GAMESVAL:-$AUTO_VAL}; GAMES1=${GAMES1:-$AUTO_G1}; GAMESN=${GAMESN:-$AUTO_GN}
log "speed from $SPEED: nodes=$NODES val=$GAMESVAL games1=$GAMES1 gamesN=$GAMESN"
gen val $GAMESVAL 9999 $NODES
gen g1 $GAMES1 101 $NODES
LIST=feat/g1.feat
train kf1 $LIST
for i in $(seq 2 $GENS); do
  p=$((i - 1))
  gen g$i $GAMESN $((i * 101)) $NODES models/kf$p.nnue
  LIST=$LIST,feat/g$i.feat
  train kf$i $LIST
done
log ALLDONE
