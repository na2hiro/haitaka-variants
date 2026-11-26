#!/bin/bash
# For each rule: build walk for base and new; run walks one at a time (15 s cap on base) and compare digests.
S=$(dirname "$0")
for r in "$@"; do
  for d in bench benchbase; do
    (cd $S/$d && cargo build --offline -q --profile walk -j 4 --bin walk --features $r --target-dir /home/na2hiro/proj/haitaka-next-rules-nps/target/walk-$d 2>&1 | grep -E "^error" -A5; cp /home/na2hiro/proj/haitaka-next-rules-nps/target/walk-$d/walk/walk walk-$r)
  done
  same=0; diff=0; skipped=0
  for bias in 1 0; do for w in $(seq 0 11); do
    b=$(timeout 15 $S/benchbase/walk-$r 1 240 $w $bias | python3 -c "import json,sys;print(json.load(sys.stdin)['digest'])" 2>/dev/null)
    n=$(timeout 60 $S/bench/walk-$r 1 240 $w $bias | python3 -c "import json,sys;print(json.load(sys.stdin)['digest'])" 2>/dev/null)
    if [ -z "$b" ]; then skipped=$((skipped+1)); [ -z "$n" ] && echo "$r walk $w bias $bias NEW ALSO TIMEOUT";
    elif [ "$b" = "$n" ]; then same=$((same+1)); else diff=$((diff+1)); echo "$r walk $w bias $bias DIFF $b $n"; fi
  done; done
  echo "$r same=$same diff=$diff baseTimeout=$skipped"
done
