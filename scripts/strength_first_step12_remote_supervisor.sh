#!/bin/bash
set -u

# Remote-owned, resumable Step 12 source-generation supervisor.
# This file is copied to the MBP and launched under caffeinate.  The output
# directory and generator command are deliberately absolute and path-consistent
# with the calibration run.

REMOTE_ROOT="/Users/na2hiro/anhoku-strength-first-step12-20260916-v1"
RECOVERY_ROOT="$REMOTE_ROOT/source/recovery"
ATTEMPT_ID="20260916T111614Z-v3"
ATTEMPT="$RECOVERY_ROOT/$ATTEMPT_ID"
LOCK="$RECOVERY_ROOT/output.lock"
OUTPUT="$REMOTE_ROOT/source/automatic-parallelism-calibration-generated"
CONFIG="$REMOTE_ROOT/source/automatic-parallelism-calibration.toml"
BINARY="$REMOTE_ROOT/bin/haitaka_learn"
STDOUT="$ATTEMPT/generation.stdout"
STDERR="$ATTEMPT/generation.stderr"
HEARTBEAT="$ATTEMPT/heartbeat.log"
EXIT_RECEIPT="$ATTEMPT/exit.json"

timestamp() {
  /bin/date -u +%Y-%m-%dT%H:%M:%SZ
}

sha256_file() {
  /usr/bin/shasum -a 256 "$1" 2>/dev/null | /usr/bin/awk '{print $1}'
}

file_bytes() {
  /usr/bin/stat -f %z "$1" 2>/dev/null || printf '0'
}

if ! /bin/mkdir -p "$RECOVERY_ROOT"; then
  exit 70
fi

if ! /bin/mkdir "$ATTEMPT" 2>/dev/null; then
  printf '%s\n' '{"schema":"haitaka-anhoku-strength-first-step12-source-recovery-startup-v1","status":"blocked-attempt-directory-exists","attemptId":"20260916T111614Z-v3"}' > "$RECOVERY_ROOT/$ATTEMPT_ID-startup-failure.json"
  exit 72
fi

if [ ! -f "$BINARY" ] || [ ! -f "$CONFIG" ] || [ ! -d "$OUTPUT" ]; then
  printf '%s\n' '{"schema":"haitaka-anhoku-strength-first-step12-source-recovery-startup-v1","status":"blocked-missing-bound-input","attemptId":"20260916T111614Z-v3"}' > "$ATTEMPT/startup-failure.json"
  exit 66
fi

if ! /bin/mkdir "$LOCK" 2>/dev/null; then
  printf '%s\n' '{"schema":"haitaka-anhoku-strength-first-step12-source-recovery-startup-v1","status":"blocked-output-lock-held","attemptId":"20260916T111614Z-v3"}' > "$ATTEMPT/startup-failure.json"
  exit 73
fi

started_at="$(timestamp)"
host_name="$(/bin/hostname)"
supervisor_pid="$$"
parent_pid="$PPID"
time_pid=""
generator_pid=""
monitor_pid=""
escalation_pid=""

printf '%s\n' "$host_name" > "$ATTEMPT/host.txt"
printf '%s\n' "$supervisor_pid" > "$ATTEMPT/supervisor.pid"
printf '%s\n' "$parent_pid" > "$ATTEMPT/parent.pid"
printf '%s\n' "$started_at" > "$ATTEMPT/started-at.txt"
printf '%s\n' "/usr/bin/time -l $BINARY generate-data --config $CONFIG --jobs 0" > "$ATTEMPT/command.txt"

on_signal() {
  requested_signal="$1"
  printf '%s\t%s\n' "$(timestamp)" "$requested_signal" >> "$ATTEMPT/signals.log"
  if [ -n "$generator_pid" ]; then
    /bin/kill -INT "$generator_pid" 2>/dev/null || true
  fi
  if [ -n "$time_pid" ]; then
    /bin/kill -INT "$time_pid" 2>/dev/null || true
  fi
  if [ "$requested_signal" = "INT" ] && [ -z "$escalation_pid" ]; then
    (
      /bin/sleep 10
      if [ -n "$generator_pid" ]; then
        /bin/kill -TERM "$generator_pid" 2>/dev/null || true
      fi
      if [ -n "$time_pid" ]; then
        /bin/kill -TERM "$time_pid" 2>/dev/null || true
      fi
    ) &
    escalation_pid="$!"
  fi
}

monitor_loop() {
  while [ -n "$time_pid" ] && /bin/kill -0 "$time_pid" 2>/dev/null; do
    observed_at="$(timestamp)"
    printf '%s\tps\n' "$observed_at" >> "$HEARTBEAT"
    if [ -n "$generator_pid" ]; then
      /bin/ps -p "$time_pid,$generator_pid" -o pid=,ppid=,state=,pcpu=,pmem=,etime=,rss=,command= >> "$HEARTBEAT" 2>&1 || true
    else
      /bin/ps -p "$time_pid" -o pid=,ppid=,state=,pcpu=,pmem=,etime=,rss=,command= >> "$HEARTBEAT" 2>&1 || true
    fi
    printf '%s\tdf\n' "$observed_at" >> "$HEARTBEAT"
    /bin/df -k "$OUTPUT" >> "$HEARTBEAT" 2>&1 || true
    /bin/sleep 30
  done
}

trap 'on_signal INT' INT
trap 'on_signal TERM' TERM

/usr/bin/time -l "$BINARY" generate-data \
  --config "$CONFIG" \
  --jobs 0 \
  < /dev/null > "$STDOUT" 2> "$STDERR" &
time_pid="$!"
printf '%s\n' "$time_pid" > "$ATTEMPT/time.pid"

for _ in $(/usr/bin/seq 1 20); do
  generator_pid="$(/usr/bin/pgrep -P "$time_pid" 2>/dev/null | /usr/bin/head -n 1 || true)"
  if [ -n "$generator_pid" ]; then
    break
  fi
  /bin/sleep 1
done
printf '%s\n' "${generator_pid:-unknown}" > "$ATTEMPT/generator.pid"

monitor_loop &
monitor_pid="$!"
printf '%s\n' "$monitor_pid" > "$ATTEMPT/monitor.pid"

wait "$time_pid"
return_code="$?"

if [ -n "$monitor_pid" ]; then
  /bin/kill "$monitor_pid" 2>/dev/null || true
  wait "$monitor_pid" 2>/dev/null || true
fi
if [ -n "$escalation_pid" ]; then
  /bin/kill "$escalation_pid" 2>/dev/null || true
  wait "$escalation_pid" 2>/dev/null || true
fi

ended_at="$(timestamp)"
status="failed"
if [ "$return_code" -eq 0 ]; then
  status="completed"
fi
stdout_bytes="$(file_bytes "$STDOUT")"
stderr_bytes="$(file_bytes "$STDERR")"
stdout_sha256="$(sha256_file "$STDOUT")"
stderr_sha256="$(sha256_file "$STDERR")"
generator_json="null"
if [ -n "$generator_pid" ]; then
  generator_json="$generator_pid"
fi

receipt_tmp="$EXIT_RECEIPT.tmp"
{
  printf '{\n'
  printf '  "schema": "haitaka-anhoku-strength-first-step12-source-recovery-v1",\n'
  printf '  "experimentId": "strength-first-step12-independent-replication-20260916-v1",\n'
  printf '  "attemptId": "20260916T111614Z-v3",\n'
  printf '  "status": "%s",\n' "$status"
  printf '  "returnCode": %s,\n' "$return_code"
  printf '  "host": "%s",\n' "$host_name"
  printf '  "supervisorPid": %s,\n' "$supervisor_pid"
  printf '  "parentPid": %s,\n' "$parent_pid"
  printf '  "timePid": %s,\n' "$time_pid"
  printf '  "generatorPid": %s,\n' "$generator_json"
  printf '  "startedAt": "%s",\n' "$started_at"
  printf '  "endedAt": "%s",\n' "$ended_at"
  printf '  "command": {\n'
  printf '    "binary": "%s",\n' "$BINARY"
  printf '    "config": "%s",\n' "$CONFIG"
  printf '    "output": "%s",\n' "$OUTPUT"
  printf '    "jobs": 0,\n'
  printf '    "shardSelector": null,\n'
  printf '    "resume": true,\n'
  printf '    "stdin": "/dev/null"\n'
  printf '  },\n'
  printf '  "logs": {\n'
  printf '    "stdout": {"path": "%s", "bytes": %s, "sha256": "%s"},\n' "$STDOUT" "$stdout_bytes" "$stdout_sha256"
  printf '    "stderr": {"path": "%s", "bytes": %s, "sha256": "%s"},\n' "$STDERR" "$stderr_bytes" "$stderr_sha256"
  printf '    "heartbeat": "%s"\n' "$HEARTBEAT"
  printf '  }\n'
  printf '}\n'
} > "$receipt_tmp" && /bin/mv "$receipt_tmp" "$EXIT_RECEIPT"

/bin/rmdir "$LOCK" 2>/dev/null || true
exit "$return_code"
