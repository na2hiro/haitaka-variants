#!/bin/bash
# usage: worker5.sh <wait-pid|0> <rule>...   waits for a pid, then runs pipeline4 for each rule
cd ~/smallnet-rules
w=$1; shift
while [ "$w" != 0 ] && kill -0 $w 2>/dev/null; do sleep 60; done
for r in "$@"; do ./pipeline4.sh $r 8 8 4 >> $r.log 2>&1; echo "$(date "+%m-%d %H:%M") $r exit $?"; done
echo ALLDONE
