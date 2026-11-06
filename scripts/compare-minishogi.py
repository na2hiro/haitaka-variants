#!/usr/bin/env python3
"""Compare movegen_oracle TSV output with an independent Fairy-Stockfish binary."""
import argparse
import json
import queue
import re
import subprocess
import threading


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", required=True)
    parser.add_argument("--positions", required=True)
    args = parser.parse_args()
    engine = subprocess.Popen([args.reference], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, bufsize=1)
    output = queue.Queue()

    def read():
        for line in engine.stdout:
            output.put(line.rstrip())
        output.put(None)

    threading.Thread(target=read, daemon=True).start()

    def send(command):
        engine.stdin.write(command + "\n")
        engine.stdin.flush()

    def until(marker):
        lines = []
        while True:
            line = output.get(timeout=30)
            if line is None:
                raise RuntimeError(f"reference engine exited before {marker}: {lines}")
            lines.append(line)
            if marker in line:
                return lines

    try:
        send("usi")
        until("usiok")
        send("setoption name UCI_Variant value minishogi")
        send("isready")
        until("readyok")
        count = 0
        with open(args.positions) as positions:
            for line in positions:
                sfen, moves = line.rstrip("\n").split("\t")
                send("position sfen " + sfen)
                send("go perft 1")
                lines = until("Nodes searched")
                reference = {match.group(1) for line in lines if (match := re.fullmatch(r"(\S+): \d+", line))}
                expected = set(moves.split())
                if reference != expected:
                    raise AssertionError({"sfen": sfen, "haitakaOnly": sorted(expected - reference),
                                          "referenceOnly": sorted(reference - expected), "output": lines})
                count += 1
        print(json.dumps({"variant": "minishogi", "positions": count, "legalMoveSetsMatch": True}))
    finally:
        if engine.poll() is None:
            send("quit")
            try:
                engine.wait(timeout=5)
            except subprocess.TimeoutExpired:
                engine.kill()
                engine.wait()


if __name__ == "__main__":
    main()
