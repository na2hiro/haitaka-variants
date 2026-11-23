#!/usr/bin/env python3
"""CPU trainer for the Anhoku KingFreeA+DonorSingleEff small net (plan phase 1/2).

Inputs are feature rows written by `haitaka_learn small-net-export-features`.
The float model mirrors haitaka_wasm's integer inference exactly up to
quantization; `export` writes the production .nnue format and `intref`
evaluates an exported file with a bit-exact integer reference so it can be
compared with `haitaka_learn small-net-eval`.

Requires torch and numpy (the variant-nnue-pytorch venv has CPU builds):
  PY=../engine/variant-nnue-pytorch/.venv/bin/python
  $PY scripts/small_net_train.py train --train a.feat --val a-val.feat --out runs/x
  $PY scripts/small_net_train.py intref --nnue runs/x/best.nnue --feat a-val.feat --limit 4096
"""

import argparse
import json
import math
import struct
import time
from pathlib import Path

import numpy as np
import torch
from torch import nn

# ---- geometry (must match haitaka_wasm::nnue) -------------------------------
SQUARES = 81
POCKETS = 18
PIECE_TYPE_COUNT = 10
NON_DROP = (2 * PIECE_TYPE_COUNT - 1) * SQUARES  # 1539
PIECE_INDICES = NON_DROP + 2 * (PIECE_TYPE_COUNT - 1) * POCKETS  # 1863
BASE = PIECE_INDICES + SQUARES  # + enemy-king plane = 1944
DONOR = SQUARES * PIECE_TYPE_COUNT * 2  # 1620
NUM_FEATURES = BASE + DONOR  # 3564
PSQT_BUCKETS = 8
LAYER_STACKS = 8
L2, L3 = 16, 32
MAX_FEATURES = 96
HEADER_BYTES = 12
NNUE2SCORE = 600.0
SCORE_CLIP = 12000

VERSION = 0x7AF32F20
KING_FREE_A_HASH = 0x4B464131
DONOR_SINGLE_ANHOKU_BLOCK_HASH = 0x23627E42 ^ 0x3C6EF362
M32 = 0xFFFFFFFF

# piece_slot order: B R S +R P L N Gold-like +B K
SLOT_VALUE = [700, 800, 400, 1000, 100, 300, 300, 520, 900, 0]
HAND_VALUE = [700, 800, 400, None, 100, 300, 300, 500, None]

HALFKA_REAL = SQUARES * PIECE_INDICES  # 150903
HALFKA_ROWS = HALFKA_REAL + DONOR  # serialized rows
HALFKA_TRAIN_ROWS = HALFKA_ROWS + PIECE_INDICES  # + factorizer virtual rows
HALFKAV2_HASH = 0x5F234CB8


def row_dtype(family):
    width, dtype = (96, "<u2") if family == "king-free" else (128, "<u4")
    return np.dtype([
        ("score", "<i2"), ("ply", "<u2"), ("stm", "u1"), ("bucket", "u1"),
        ("nb", "u1"), ("nw", "u1"), ("result", "i1"), ("pad", "u1", 3),
        ("black", dtype, width), ("white", dtype, width),
    ])


FAMILY = "king-free"


def train_rows():
    return NUM_FEATURES if FAMILY == "king-free" else HALFKA_TRAIN_ROWS


def serialized_rows():
    return NUM_FEATURES if FAMILY == "king-free" else HALFKA_ROWS


def composite(head, tail):
    return (head ^ ((tail << 1) & M32) ^ (tail >> 1)) & M32


def output_layer_hash(l1):
    def affine(prev, out):
        h = (0xCC03DAE4 + out) & M32
        h ^= prev >> 1
        h ^= (prev << 31) & M32
        return h

    def relu(prev):
        return (0x538D24C7 + prev) & M32

    h = 0xEC42E90D ^ (2 * l1)
    h = relu(affine(h, L2))
    h = relu(affine(h, L3))
    return affine(h, 1)


def hashes(l1):
    head = KING_FREE_A_HASH if FAMILY == "king-free" else HALFKAV2_HASH
    fs = composite(head, DONOR_SINGLE_ANHOKU_BLOCK_HASH)
    ft = fs ^ (2 * l1)
    out = output_layer_hash(l1)
    return ft ^ out, ft, out


# ---- data -------------------------------------------------------------------
class FeatureSet:
    def __init__(self, path, max_ply=None, max_abs_score=None):
        # comma-separated paths are concatenated (avoids writing combined copies)
        rows = np.concatenate([np.fromfile(p, dtype=row_dtype(FAMILY)) for p in str(path).split(",")])
        keep = np.ones(len(rows), dtype=bool)
        if max_ply is not None:
            keep &= rows["ply"] < max_ply
        if max_abs_score is not None:
            keep &= np.abs(rows["score"].astype(np.int32)) < max_abs_score
        rows = rows[keep]
        self.n = len(rows)
        self.score = torch.from_numpy(np.clip(rows["score"].astype(np.float32), -SCORE_CLIP, SCORE_CLIP))
        self.stm = torch.from_numpy(rows["stm"].astype(np.int64))
        self.bucket = torch.from_numpy(rows["bucket"].astype(np.int64))
        # int32 keeps a 5M-row corpus within a few GB; EmbeddingBag accepts int32
        self.black = torch.from_numpy(rows["black"].astype(np.int32))
        self.white = torch.from_numpy(rows["white"].astype(np.int32))
        self.nb = torch.from_numpy(rows["nb"].astype(np.int64))
        self.nw = torch.from_numpy(rows["nw"].astype(np.int64))

    def batch(self, idx):
        return (self.black[idx], self.nb[idx], self.white[idx], self.nw[idx],
                self.stm[idx], self.bucket[idx], self.score[idx])


def bag_inputs(indices, counts):
    width = indices.shape[1]
    mask = torch.arange(width).unsqueeze(0) < counts.unsqueeze(1)
    flat = indices[mask]
    offsets = torch.zeros(len(counts), dtype=indices.dtype)
    offsets[1:] = torch.cumsum(counts, 0)[:-1].to(indices.dtype)
    return flat, offsets


# ---- model ------------------------------------------------------------------
class SmallNet(nn.Module):
    def __init__(self, l1, seed, init="small"):
        super().__init__()
        g = torch.Generator().manual_seed(seed)
        self.l1 = l1
        self.ft = nn.EmbeddingBag(train_rows(), l1 + PSQT_BUCKETS, mode="sum")
        self.ft_bias = nn.Parameter(torch.full((l1,), 0.25))
        self.h1 = nn.Linear(2 * l1, L2 * LAYER_STACKS)
        self.h2 = nn.Linear(L2, L3 * LAYER_STACKS)
        self.out = nn.Linear(L3, LAYER_STACKS)
        with torch.no_grad():
            w = self.ft.weight
            w[:, :l1].uniform_(-0.05, 0.05, generator=g)
            mat = torch.from_numpy(material_psqt()[:PIECE_INDICES])
            w[:, l1:] = 0.0
            if FAMILY == "king-free":
                w[:PIECE_INDICES, l1:] = mat.unsqueeze(1).expand(-1, PSQT_BUCKETS)
            else:
                # factorizer: real king-bucket rows start as pure zero residuals
                w[:HALFKA_REAL, :] = 0.0
                w[HALFKA_ROWS:, l1:] = mat.unsqueeze(1).expand(-1, PSQT_BUCKETS)
            for layer, fan_in in ((self.h1, 2 * l1), (self.h2, L2), (self.out, L3)):
                bound = 1.0 / math.sqrt(fan_in)
                layer.weight.uniform_(-bound, bound, generator=g)
                layer.bias.uniform_(-bound, bound, generator=g)
            # identical stacks at init, and a quiet output layer
            for i in range(1, LAYER_STACKS):
                self.h1.weight[i * L2:(i + 1) * L2] = self.h1.weight[:L2]
                self.h1.bias[i * L2:(i + 1) * L2] = self.h1.bias[:L2]
                self.h2.weight[i * L3:(i + 1) * L3] = self.h2.weight[:L3]
                self.h2.bias[i * L3:(i + 1) * L3] = self.h2.bias[:L3]
            if init == "small":
                self.out.weight.mul_(0.1)
            self.out.weight[1:] = self.out.weight[:1]
            self.out.bias.zero_()
            if init == "nnue":
                # variant-nnue-pytorch: FT weights/bias U(+-sqrt(1/rows)), virtual
                # (factorizer) rows zero, PSQT = material, PSQT bias absent.
                sigma = math.sqrt(1.0 / train_rows())
                w[:, :l1].uniform_(-sigma, sigma, generator=g)
                self.ft_bias.uniform_(-sigma, sigma, generator=g)
                if FAMILY == "halfka":
                    w[HALFKA_ROWS:, :] = 0.0
                    w[:HALFKA_REAL, l1:] = mat.repeat(SQUARES).unsqueeze(1).expand(-1, PSQT_BUCKETS)

    def forward(self, black, nb, white, nw, stm, bucket):
        fb, ob = bag_inputs(black, nb)
        fw, ow = bag_inputs(white, nw)
        ab = self.ft(fb, ob)
        aw = self.ft(fw, ow)
        black_to_move = (stm == 0).unsqueeze(1)
        us = torch.where(black_to_move, ab, aw)
        them = torch.where(black_to_move, aw, ab)
        l1 = self.l1
        x = torch.clamp(torch.cat([us[:, :l1], them[:, :l1]], 1) + torch.cat([self.ft_bias, self.ft_bias]), 0.0, 1.0)
        b = bucket
        rows = torch.arange(len(b))
        h1 = self.h1(x).view(-1, LAYER_STACKS, L2)[rows, b]
        h1 = torch.clamp(h1, 0.0, 1.0)
        h2 = self.h2(h1).view(-1, LAYER_STACKS, L3)[rows, b]
        h2 = torch.clamp(h2, 0.0, 1.0)
        o = self.out(h2)[rows, b]
        psqt = (us[:, l1:].gather(1, b.unsqueeze(1)) - them[:, l1:].gather(1, b.unsqueeze(1))).squeeze(1) / 2
        return (o + psqt) * NNUE2SCORE

    def clip_(self):
        with torch.no_grad():
            self.h1.weight.clamp_(-127 / 64, 127 / 64)
            self.h2.weight.clamp_(-127 / 64, 127 / 64)
            self.out.weight.clamp_(-127 * 127 / 9600, 127 * 127 / 9600)


def material_psqt():
    """Per-row PSQT (net units) so an untrained net returns the material balance."""
    v = np.zeros(NUM_FEATURES, dtype=np.float32)
    for slot, value in enumerate(SLOT_VALUE):
        for color_offset, sign in ((0, 1), (1, -1)):
            if slot == 9 and color_offset == 1:
                continue  # one king plane in the base block
            start = (2 * slot + color_offset) * SQUARES
            v[start:start + SQUARES] = sign * value / NNUE2SCORE
    for slot, value in enumerate(HAND_VALUE):
        if value is None:
            continue
        for color_offset, sign in ((0, 1), (1, -1)):
            start = NON_DROP + (2 * slot + color_offset) * POCKETS
            v[start:start + POCKETS] = sign * value / NNUE2SCORE
    return v


LOSS = "sigmoid"


def loss_fn(pred, target, k):
    if LOSS == "huber":
        # F1 arm C / step10 recipe: normalized clipped-score Huber, delta 0.1.
        e = (pred - target) / SCORE_CLIP
        a = e.abs()
        return torch.where(a <= 0.1, 0.5 * e * e, 0.1 * (a - 0.05)).mean()
    return ((torch.sigmoid(pred / k) - torch.sigmoid(target / k)) ** 2).mean()


def evaluate(model, data, k, batch=32768):
    model.eval()
    total, mae, n = 0.0, 0.0, 0
    with torch.no_grad():
        for start in range(0, data.n, batch):
            idx = torch.arange(start, min(start + batch, data.n))
            *inp, target = data.batch(idx)
            pred = model(*inp)
            total += loss_fn(pred, target, k).item() * len(idx)
            mae += (pred - target).abs().sum().item()
            n += len(idx)
    model.train()
    return total / n, mae / n


# ---- export -----------------------------------------------------------------
def coalesced_ft(model):
    w = model.ft.weight.detach()
    if FAMILY == "king-free":
        return w
    real = w[:HALFKA_ROWS].clone()
    virtual = w[HALFKA_ROWS:]
    real[:HALFKA_REAL] += virtual.repeat(SQUARES, 1)
    return real


def export_nnue(model, path, description):
    l1 = model.l1
    net_hash, ft_hash, out_hash = hashes(l1)
    buf = bytearray()
    buf += struct.pack("<II", VERSION, net_hash)
    desc = description.encode()
    buf += struct.pack("<I", len(desc)) + desc
    buf += struct.pack("<I", ft_hash)
    w = coalesced_ft(model)
    bias = torch.round(model.ft_bias.detach() * 127)
    ftw = torch.round(w[:, :l1] * 127)
    assert bias.abs().max() <= 32767 and ftw.abs().max() <= 32767
    buf += bias.to(torch.int16).numpy().tobytes()
    buf += ftw.to(torch.int16).numpy().tobytes()
    buf += torch.round(w[:, l1:] * 9600).to(torch.int32).numpy().tobytes()
    for i in range(LAYER_STACKS):
        buf += struct.pack("<I", out_hash)
        for layer, outs, rows, is_out in (
            (model.h1, L2, slice(i * L2, (i + 1) * L2), False),
            (model.h2, L3, slice(i * L3, (i + 1) * L3), False),
            (model.out, 1, slice(i, i + 1), True),
        ):
            bias_scale = 9600.0 if is_out else 64 * 127.0
            weight_scale = bias_scale / 127.0
            lb = torch.round(layer.bias.detach()[rows] * bias_scale).to(torch.int32)
            lw = torch.round(layer.weight.detach()[rows].clamp(-127 / weight_scale, 127 / weight_scale) * weight_scale)
            inputs = lw.shape[1]
            padded = ((inputs + 31) // 32) * 32
            full = torch.zeros(outs, padded)
            full[:, :inputs] = lw
            buf += lb.numpy().tobytes()
            buf += full.to(torch.int8).numpy().tobytes()
    Path(path).write_bytes(bytes(buf))
    return len(buf)


def read_nnue(path, l1):
    data = Path(path).read_bytes()
    off = 0

    def take(fmt_dtype, count):
        nonlocal off
        arr = np.frombuffer(data, dtype=fmt_dtype, count=count, offset=off)
        off += arr.nbytes
        return arr

    version, net_hash = take("<u4", 2)
    assert version == VERSION and net_hash == hashes(l1)[0], "not a KingFreeA network of this width"
    (dlen,) = take("<u4", 1)
    off += int(dlen)
    take("<u4", 1)
    ftb = take("<i2", l1).astype(np.int32)
    nrows = serialized_rows()
    ftw = take("<i2", nrows * l1).reshape(nrows, l1).astype(np.int32)
    psqt = take("<i4", nrows * PSQT_BUCKETS).reshape(nrows, PSQT_BUCKETS).astype(np.int64)
    stacks = []
    for _ in range(LAYER_STACKS):
        take("<u4", 1)
        layers = []
        for outs, padded in ((L2, 2 * l1), (L3, 32), (1, 32)):
            b = take("<i4", outs).astype(np.int64)
            w = take("i1", outs * padded).reshape(outs, padded).astype(np.int64)
            layers.append((b, w))
        stacks.append(layers)
    assert off == len(data)
    return ftb, ftw, psqt, stacks


def trunc_div(a, b):
    q = np.abs(a) // b
    return np.where(a < 0, -q, q)


def integer_reference(path, data, l1, limit):
    ftb, ftw, psqt, stacks = read_nnue(path, l1)
    nrows = serialized_rows()
    n = min(limit, data.n)
    out = np.zeros(n, dtype=np.int64)
    for i in range(n):
        acc, ps = [], []
        for idx, cnt in ((data.black[i], data.nb[i]), (data.white[i], data.nw[i])):
            rows = idx[: int(cnt)].numpy()
            rows = rows[rows < nrows]  # drop factorizer virtual rows
            a = ftb + ftw[rows].sum(0)
            assert np.all(np.abs(a) <= 32767)
            acc.append(a)
            ps.append(psqt[rows].sum(0))
        us = 0 if int(data.stm[i]) == 0 else 1
        x = np.concatenate([np.clip(acc[us], 0, 127), np.clip(acc[1 - us], 0, 127)]).astype(np.int64)
        b = int(data.bucket[i])
        (b1, w1), (b2, w2), (b3, w3) = stacks[b]
        h1 = np.clip((w1 @ x + b1) >> 6, 0, 127)
        h1p = np.zeros(32, dtype=np.int64)
        h1p[:L2] = h1
        h2 = np.clip((w2 @ h1p + b2) >> 6, 0, 127)
        o = int((w3 @ h2 + b3)[0])
        p = int(trunc_div(np.int64(ps[us][b] - ps[1 - us][b]), 2))
        out[i] = int(trunc_div(np.int64(p + o), 16))
    return out


# ---- commands ---------------------------------------------------------------
def cmd_train(args):
    global LOSS, FAMILY
    LOSS = args.loss
    FAMILY = args.family
    torch.manual_seed(args.seed)
    torch.set_num_threads(args.threads)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    train = FeatureSet(args.train, args.max_ply, args.max_abs_score)
    val = FeatureSet(args.val)
    model = SmallNet(args.l1, args.seed, args.init)
    groups = [
        {"params": list(model.ft.parameters()) + [model.ft_bias], "lr": args.lr},
        {"params": list(model.h1.parameters()) + list(model.h2.parameters()), "lr": args.lr},
        {"params": list(model.out.parameters()), "lr": args.lr * args.output_lr_mult},
    ]
    if args.optimizer == "ranger":
        import sys
        sys.path.insert(0, args.ranger_dir)
        import ranger
        opt = ranger.Ranger(groups, betas=(.9, 0.999), eps=1.0e-7, gc_loc=False, use_gc=False)
    else:
        opt = torch.optim.Adam(groups, lr=args.lr)
    steps_per_epoch = max(1, train.n // args.batch)
    total_steps = args.max_steps or steps_per_epoch * args.epochs
    horizon = args.horizon or total_steps
    min_mult = args.lr_floor

    def lr_mult(s):
        if args.warmup and s < args.warmup:
            return (s + 1) / args.warmup
        t = min(1.0, (s - args.warmup) / max(1, horizon - args.warmup))
        return min_mult + (1 - min_mult) * 0.5 * (1 + math.cos(math.pi * t))

    sched = torch.optim.lr_scheduler.LambdaLR(opt, lr_mult)
    eval_every = args.eval_every or steps_per_epoch
    log = []
    init_loss, init_mae = evaluate(model, val, args.k)
    log.append({"epoch": 0, "step": 0, "valLoss": init_loss, "valMae": init_mae})
    print(f"init val loss {init_loss:.6f} mae {init_mae:.1f} (train rows {train.n}, val rows {val.n})", flush=True)
    best = (float("inf"), 0)
    gen = torch.Generator().manual_seed(args.seed + 1)
    step = 0
    started = time.time()
    running, running_n, epoch = 0.0, 0, 0
    perm, pos = None, train.n
    while step < total_steps:
        if pos + args.batch > train.n:
            perm, pos, epoch = torch.randperm(train.n, generator=gen), 0, epoch + 1
        idx = perm[pos:pos + args.batch]
        pos += args.batch
        *inp, target = train.batch(idx)
        model.clip_()
        loss = loss_fn(model(*inp), target, args.k)
        opt.zero_grad(set_to_none=True)
        loss.backward()
        opt.step()
        sched.step()
        model.clip_()
        running += loss.item()
        running_n += 1
        step += 1
        if step % eval_every == 0 or step == total_steps:
            vloss, vmae = evaluate(model, val, args.k)
            entry = {"epoch": epoch, "step": step, "trainLoss": running / running_n,
                     "valLoss": vloss, "valMae": vmae, "lr": sched.get_last_lr()[0],
                     "elapsedSec": time.time() - started}
            running, running_n = 0.0, 0
            log.append(entry)
            print(json.dumps(entry), flush=True)
            if args.keep_every and step % args.keep_every == 0:
                torch.save(model.state_dict(), out / f"step{step:05d}.pt")
            if vloss < best[0] and (not args.select_cap or step <= args.select_cap):
                best = (vloss, step)
                torch.save(model.state_dict(), out / "best.pt")
            if args.patience and (step - best[1]) >= args.patience * eval_every:
                break
    torch.save(model.state_dict(), out / "last.pt")
    model.load_state_dict(torch.load(out / "best.pt"))
    size = export_nnue(model, out / "best.nnue", f"{FAMILY} L1={args.l1} {out.name} step={best[1]}")
    summary = {"args": {k: v for k, v in vars(args).items() if k != "fn"}, "bestStep": best[1], "bestValLoss": best[0], "nnueBytes": size,
               "trainRows": train.n, "valRows": val.n, "log": log}
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(f"best step {best[1]} val loss {best[0]:.6f}; wrote {out/'best.nnue'} ({size} bytes)")


def cmd_intref(args):
    global FAMILY
    FAMILY = args.family
    data = FeatureSet(args.feat)
    ref = integer_reference(args.nnue, data, args.l1, args.limit)
    if args.rust:
        rust = np.fromfile(args.rust, dtype="<i4")[: len(ref)]
        mism = int(np.sum(rust != ref))
        print(f"integer reference vs Rust: {mism} mismatches / {len(ref)}")
    if args.checkpoint:
        model = SmallNet(args.l1, 0)
        model.load_state_dict(torch.load(args.checkpoint))
        model.eval()
        with torch.no_grad():
            idx = torch.arange(len(ref))
            *inp, _ = data.batch(idx)
            fp = model(*inp).numpy()
        d = np.abs(fp - ref)
        print(f"float vs integer: MAE {d.mean():.2f}, p95 {np.percentile(d, 95):.2f}, max {d.max():.2f}")
    if args.output:
        ref.astype("<i4").tofile(args.output)


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("train")
    t.add_argument("--train", required=True)
    t.add_argument("--val", required=True)
    t.add_argument("--out", required=True)
    t.add_argument("--l1", type=int, default=256)
    t.add_argument("--epochs", type=int, default=40)
    t.add_argument("--batch", type=int, default=16384)
    t.add_argument("--lr", type=float, default=1e-3)
    t.add_argument("--lr-floor", type=float, default=0.02)
    t.add_argument("--k", type=float, default=1000.0, help="sigmoid scale of the score loss")
    t.add_argument("--loss", choices=["sigmoid", "huber"], default="sigmoid")
    t.add_argument("--family", choices=["king-free", "halfka"], default="king-free")
    t.add_argument("--init", choices=["small", "nnue"], default="small")
    t.add_argument("--optimizer", choices=["adam", "ranger"], default="adam")
    t.add_argument("--ranger-dir", default="/home/na2hiro/proj/engine/variant-nnue-pytorch")
    t.add_argument("--output-lr-mult", type=float, default=1.0)
    t.add_argument("--warmup", type=int, default=0)
    t.add_argument("--horizon", type=int, default=0, help="cosine horizon in steps (default: run length)")
    t.add_argument("--max-steps", type=int, default=0)
    t.add_argument("--eval-every", type=int, default=0)
    t.add_argument("--keep-every", type=int, default=0)
    t.add_argument("--select-cap", type=int, default=0, help="only select checkpoints at or before this step")
    t.add_argument("--seed", type=int, default=1)
    t.add_argument("--threads", type=int, default=4)
    t.add_argument("--patience", type=int, default=0)
    t.add_argument("--max-ply", type=int, default=None)
    t.add_argument("--max-abs-score", type=int, default=None)
    t.set_defaults(fn=cmd_train)
    r = sub.add_parser("intref")
    r.add_argument("--nnue", required=True)
    r.add_argument("--feat", required=True)
    r.add_argument("--l1", type=int, default=256)
    r.add_argument("--family", choices=["king-free", "halfka"], default="king-free")
    r.add_argument("--limit", type=int, default=4096)
    r.add_argument("--rust")
    r.add_argument("--checkpoint")
    r.add_argument("--output")
    r.set_defaults(fn=cmd_intref)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
