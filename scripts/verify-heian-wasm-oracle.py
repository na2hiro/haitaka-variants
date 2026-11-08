#!/usr/bin/env python3
"""Build a verification-only WASM wrapper for all Heian Native TSV transitions."""
from pathlib import Path
import argparse,json,os,shutil,subprocess,tomllib
ROOT=Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--wasm-bindgen',default=shutil.which('wasm-bindgen'));p.add_argument('--output',type=Path,required=True);args=p.parse_args()
if not args.wasm_bindgen:p.error('specify --wasm-bindgen PATH')
work=ROOT/'target/heian-wasm-oracle';(work/'src').mkdir(parents=True,exist_ok=True)
shutil.copy2(ROOT/'scripts/heian-wasm-oracle.rs',work/'src/lib.rs');shutil.copy2(ROOT/'Cargo.lock',work/'Cargo.lock')
version=next(q['version'] for q in tomllib.loads((ROOT/'Cargo.lock').read_text())['package'] if q['name']=='wasm-bindgen')
rules=['heian-8x8','heian-9x8','heian-9x9']
manifest='[package]\nname="heian-wasm-oracle"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[lib]\ncrate-type=["cdylib"]\n[dependencies]\nhaitaka={path='+json.dumps(str(ROOT/'haitaka'))+',default-features=false}\nwasm-bindgen="='+version+'"\n[profile.release]\nopt-level=3\ncodegen-units=1\nlto="fat"\n[features]\n'
for rule in rules:manifest+=rule+'=["haitaka/'+rule+'"]\n'
(work/'Cargo.toml').write_text(manifest)
for rule in rules:
 with (work/(rule+'-build.log')).open('w') as log:
  subprocess.run(['cargo','build','--offline','--release','--manifest-path',str(work/'Cargo.toml'),'--target-dir',str(ROOT/'target'),'--target','wasm32-unknown-unknown','--features',rule],cwd=ROOT,env={**os.environ,'RUSTFLAGS':'-C target-feature=-simd128'},stdout=log,stderr=log,check=True)
 subprocess.run([args.wasm_bindgen,'--target','web','--out-name','oracle','--out-dir',str(work/rule),str(ROOT/'target/wasm32-unknown-unknown/release/heian_wasm_oracle.wasm')],check=True)
 print('built',rule,flush=True)
subprocess.run(['node',str(ROOT/'scripts/verify-heian-wasm-oracle.mjs'),str(ROOT/'target'),str(args.output)],cwd=ROOT,check=True)
