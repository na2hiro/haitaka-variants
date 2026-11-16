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
    parser.add_argument("--variant", default="minishogi")
    parser.add_argument("--variant-file")
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
        if args.variant_file:
            send("load " + args.variant_file)
        send("setoption name UCI_Variant value " + args.variant)
        send("isready")
        until("readyok")
        count = 0
        pawn_drop_mates = 0
        with open(args.positions) as positions:
            for line in positions:
                sfen, moves = line.rstrip("\n").split("\t")
                send("position sfen " + sfen)
                send("go perft 1")
                lines = until("Nodes searched")
                reference = {match.group(1) for line in lines if (match := re.fullmatch(r"(\S+): \d+", line))}
                # Fairy includes uchifuzume in its legal generator and scores the
                # resulting mate as a loss for the dropper. Shogitter excludes it.
                placement, side, *_ = sfen.split()
                enemy_king = "k" if side == "b" else "K"
                king = None
                height = len(placement.split("/"))
                for rank, row in enumerate(placement.split("/")):
                    width = sum(int(ch) if ch.isdigit() else ch != "+" for ch in row)
                    file = width
                    for ch in row:
                        if ch == "+":
                            continue
                        if ch.isdigit():
                            file -= int(ch)
                        else:
                            if ch == enemy_king:
                                king = (file, rank)
                            file -= 1
                if king is not None:
                    file, rank = king
                    pawn_rank = rank + (1 if side == "b" else -1)
                    checking_drop = f"P*{file}{chr(ord('a') + pawn_rank)}"
                    if 0 <= pawn_rank < height and checking_drop in reference:
                        send("position sfen " + sfen + " moves " + checking_drop)
                        send("go perft 1")
                        replies = until("Nodes searched")
                        if "Nodes searched: 0" in replies:
                            reference.remove(checking_drop)
                            pawn_drop_mates += 1
                expected = set(moves.split())
                if reference != expected:
                    raise AssertionError({"sfen": sfen, "haitakaOnly": sorted(expected - reference),
                                          "referenceOnly": sorted(reference - expected), "output": lines})
                count += 1
        print(json.dumps({"variant": args.variant, "positions": count, "legalMoveSetsMatch": True, "excludedFairyPawnDropMates": pawn_drop_mates}))
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
