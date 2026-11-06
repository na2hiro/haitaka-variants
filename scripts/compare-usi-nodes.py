#!/usr/bin/env python3
"""Alternate two native USI builds at identical fixed-node search budgets."""
import argparse
import json
import queue
import re
import statistics
import subprocess
import threading
import time

POSITIONS = [
    ("startpos", "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"),
    ("endgame", "ln1g5/1r4k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b b BG2SLPnp 61"),
]


class Engine:
    def __init__(self, binary):
        self.process = subprocess.Popen([binary, "usi"], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                        text=True, bufsize=1)
        self.output = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        self.send("usi")
        self.until("usiok")

    def _read(self):
        for line in self.process.stdout:
            self.output.put(line.strip())
        self.output.put(None)

    def send(self, command):
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()

    def until(self, prefix):
        lines = []
        while True:
            line = self.output.get(timeout=60)
            if line is None:
                raise RuntimeError(f"engine exited: {lines}")
            lines.append(line)
            if line.startswith(prefix):
                return lines

    def search(self, sfen, nodes):
        self.send("usinewgame\nposition sfen " + sfen + "\nisready")
        self.until("readyok")
        start = time.perf_counter_ns()
        self.send(f"go nodes {nodes}")
        lines = self.until("bestmove ")
        elapsed = (time.perf_counter_ns() - start) / 1_000_000
        info = next(line for line in reversed(lines) if line.startswith("info depth "))
        fields = {name: match.group(1) if (match := re.search(rf"\b{name} (\S+)", info)) else None
                  for name in ["depth", "nodes", "qnodes", "lastCompletedIterationValue"]}
        return {"ms": elapsed, "bestmove": lines[-1], "fields": fields}

    def close(self):
        if self.process.poll() is None:
            self.send("quit")
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--before", required=True)
    parser.add_argument("--after", required=True)
    parser.add_argument("--nodes", type=int, default=10000)
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--output")
    args = parser.parse_args()
    if args.nodes <= 0 or args.samples <= 0:
        parser.error("nodes and samples must be positive")
    before = Engine(args.before)
    after = None
    try:
        after = Engine(args.after)
        results = []
        for name, sfen in POSITIONS:
            for _ in range(4):
                before.search(sfen, args.nodes)
                after.search(sfen, args.nodes)
            samples = []
            for index in range(args.samples):
                if index % 2:
                    a = after.search(sfen, args.nodes)
                    b = before.search(sfen, args.nodes)
                else:
                    b = before.search(sfen, args.nodes)
                    a = after.search(sfen, args.nodes)
                assert b["bestmove"] == a["bestmove"], (name, b, a)
                assert b["fields"] == a["fields"], (name, b, a)
                samples.append({"beforeMs": b["ms"], "afterMs": a["ms"]})
            results.append({"name": name, "nodes": args.nodes,
                            "beforeMedianMs": statistics.median(s["beforeMs"] for s in samples),
                            "afterMedianMs": statistics.median(s["afterMs"] for s in samples),
                            "medianPairedChangePercent": statistics.median(
                                (s["afterMs"] / s["beforeMs"] - 1) * 100 for s in samples),
                            "searchResultsMatch": True, "samples": samples})
        if args.output:
            with open(args.output, "w") as output:
                json.dump(results, output, indent=2)
                output.write("\n")
        print(json.dumps([{k: v for k, v in result.items() if k != "samples"} for result in results]))
    finally:
        before.close()
        if after:
            after.close()


if __name__ == "__main__":
    main()
