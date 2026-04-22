# Shogitter Engine Package v1 Implementation Plan
                                                                                                                    
## Summary                                                                                                          
                                                                                                                    
Define a Shogitter-owned engine package contract and implement it end to end across both repositories. Shogitter    
should support two runtime models:                                                                                  
                                                                                                                    
- wasm-bindgen: first-party/direct WASM engines such as Haitaka.                                                    
- emscripten-usi: legacy/protocol engines such as Fairy-Stockfish and YaneuraOu.                                    
                                                                                                                    
Haitaka should package its current wasm-pack output directly with a shogitter-engine.json manifest. Shogitter       
should use that manifest as the authoritative source for runtime type, artifact paths, and supported rules, while   
preserving the existing heuristic upload path for legacy engines.                                                   
                                                                                                                    
## Shogitter Changes                                                                                                
                                                                                                                    
### Manifest Contract                                                                                               
                                                                                                                    
Support a root-level archive manifest:                                                                              
                                                                                                                    
shogitter-engine.json                                                                                               
                                                                                                                    
Manifest v1:                                                                                                        
                                                                                                                    
{                                                                                                                   
  "schema": "shogitter-engine-package",                                                                             
  "schemaVersion": 1,                                                                                               
  "engine": {                                                                                                       
    "id": "haitaka-variants",                                                                                       
    "name": "Haitaka Variants",                                                                                     
    "version": "0.1.0",                                                                                             
    "commit": "..."                                                                                                 
  },                                                                                                                
  "runtime": {                                                                                                      
    "kind": "wasm-bindgen",                                                                                         
    "module": "engine/haitaka_wasm.js",                                                                             
    "wasm": "engine/haitaka_wasm_bg.wasm"                                                                           
  },                                                                                                                
  "capabilities": {                                                                                                 
    "protocols": ["shogitter-direct-v1"],                                                                           
    "commands": ["search", "iterative-search", "perft", "dfpn"],                                                    
    "supportsPonder": false,                                                                                        
    "supportsMovetime": true,                                                                                       
    "supportsDepth": true                                                                                           
  },                                                                                                                
  "rules": [                                                                                                        
    {                                                                                                               
      "ruleId": 26,                                                                                                 
      "variant": "annan",                                                                                           
      "positionFormat": "sfen",                                                                                     
      "moveFormat": "usi",                                                                                          
      "startpos": "..."                                                                                             
    }                                                                                                               
  ],                                                                                                                
  "artifacts": {                                                                                                    
    "nnue": null                                                                                                    
  }                                                                                                                 
}                                                                                                                   
                                                                                                                    
Supported runtime kinds:                                                                                            
                                                                                                                    
type EngineRuntimeKind = "wasm-bindgen" | "emscripten-usi";                                                         
                                                                                                                    
### Extraction And Stored Types                                                                                     
                                                                                                                    
Update the current tarball extraction flow.                                                                         
                                                                                                                    
Behavior:                                                                                                           
                                                                                                                    
- If shogitter-engine.json exists:                                                                                  
    - parse and validate schema === "shogitter-engine-package"                                                      
    - require schemaVersion === 1                                                                                   
    - resolve artifact paths relative to archive root                                                               
    - for runtime.kind === "wasm-bindgen" require:                                                                  
        - manifest-declared JS module exists                                                                        
        - manifest-declared WASM exists                                                                             
    - for runtime.kind === "emscripten-usi" require:                                                                
        - loader JS exists                                                                                          
        - WASM exists                                                                                               
        - worker JS exists                                                                                          
    - return a bundle containing the manifest and runtime kind                                                      
- If no manifest exists:                                                                                            
    - preserve current legacy heuristic:                                                                            
        - exactly one .wasm                                                                                         
        - matching <base>.js                                                                                        
        - matching <base>.worker.js                                                                                 
        - optional .nnue, nn.bin, or eval.bin                                                                       
    - treat as runtimeKind = "emscripten-usi"                                                                       
                                                                                                                    
Update stored descriptor types so engines can store:                                                                
                                                                                                                    
- runtimeKind                                                                                                       
- optional parsed manifest                                                                                          
- wasm                                                                                                              
- for legacy engines: loaderJs, workerJs                                                                            
- for wasm-bindgen engines: wasmBindgenModuleJs                                                                     
- optional NNUE artifact                                                                                            
- capabilities                                                                                                      
- rule mappings                                                                                                     
                                                                                                                    
Backward compatibility: existing IndexedDB descriptors without runtimeKind should be treated as emscripten-usi.                                                                                                                      
### Adapter Layer                                                                                                   
                                                                                                                    
Add a common internal engine adapter interface:                                                                     
                                                                                                                    
type EngineSearchArgs = {                                                                                           
  ruleId: number;                                                                                                   
  position: string;                                                                                                 
  positionFormat: "sfen" | "fen";                                                                                   
  moveTimeMs?: number;                                                                                              
  depth?: number;                                                                                                   
};                                                                                                                  
                                                                                                                    
type EngineSearchResult = {                                                                                         
  bestMove: string | null;                                                                                          
  ponder?: string;                                                                                                  
  stats?: {                                                                                                         
    nodes?: number;                                                                                                 
    nps?: number;                                                                                                   
    elapsedMs?: number;                                                                                             
    depth?: number;                                                                                                 
  };                                                                                                                
};                                                                                                                  
                                                                                                                    
type EngineAdapter = {                                                                                              
  init(): Promise<void>;                                                                                            
  search(args: EngineSearchArgs): Promise<EngineSearchResult>;                                                      
  perft?(args: {                                                                                                    
    ruleId: number;                                                                                                 
    position: string;                                                                                               
    positionFormat: "sfen" | "fen";                                                                                 
    depth: number;                                                                                                  
  }): Promise<{ nodes: number; elapsedMs?: number; nps?: number }>;                                                 
  stop?(): void;                                                                                                    
  dispose(): void;                                                                                                  
};                                                                                                                  
                                                                                                                    
Implement:                                                                                                          
                                                                                                                    
- EmscriptenUsiEngineAdapter                                                                                        
    - wraps the existing engineWorkerScript                                                                         
    - preserves current behavior:                                                                                   
        - usi                                                                                                       
        - isready                                                                                                   
        - setoption name VariantPath value /variants.ini                                                            
        - setoption name UCI_Variant value ...                                                                      
        - position sfen|fen ...                                                                                     
        - go movetime ...                                                                                           
        - go ponder                                                                                                 
        - ponderhit                                                                                                 
        - stop                                                                                                      
        - go perft ...                                                                                              
        - parse bestmove ...                                                                                        
- WasmBindgenEngineAdapter                                                                                          
    - loads the manifest-declared JS module and WASM                                                                
    - calls the wasm-bindgen default initializer                                                                    
    - calls:                                                                                                        
        - search_iterative_deepening(sfen, maxDepth, timeoutMs) when available                                      
        - search(sfen, depth) as fallback                                                                           
        - perft(sfen, depth) when available                                                                         
    - returns USI bestMove unchanged                                                                                
    - v1 does not need ponder support                                                                               
                                                                                                                    
### Registration And Probe Flow                                                                                     
                                                                                                                    
Update /my/engines behavior:                                                                                        
                                                                                                                    
- For manifest packages:                                                                                            
    - skip UCI_Variant rule inference                                                                               
    - use manifest.rules as authoritative                                                                           
    - create ruleMappings from manifest rules                                                                       
    - display runtime kind, engine name, version, and declared rules                                                
    - run a smoke init using the selected adapter                                                                   
- For legacy packages:                                                                                              
    - preserve current probe behavior                                                                               
    - parse option name ...                                                                                         
    - infer variants from UCI_Variant                                                                               
    - infer Fairy-Stockfish compatibility from VariantPath                                                          
    - preserve current rule mapping behavior                                                                        
                                                                                                                    
Update probe naming/structure so the current probe path is clearly the legacy emscripten-usi probe, while manifest  
packages use a smoke check instead.                                                                                 
                                                                                                                    
### User Engine Runtime                   
Update user-engine execution to select the adapter by descriptor runtime:                                           
                                                                                                                    
- emscripten-usi: use the current protocol behavior and existing move-format sniffing.                              
- wasm-bindgen: use direct module calls.                                                                            
                                                                                                                    
For wasm-bindgen packages:                                                                                          
                                                                                                                    
- serialize positions using the manifest rule entry:                                                                
    - Haitaka v1 uses positionFormat = "sfen"                                                                       
- parse returned moves using:                                                                                       
    - Haitaka v1 uses moveFormat = "usi"                                                                            
- for unsupported manifest formats, fail with a clear error.                                                        
                                                                                                                    
## Haitaka Changes                                                                                                  
                                                                                                                    
### Package Generation                                                                                              
                                                                                                                    
Update haitaka_cli package to emit a Shogitter Engine Package v1 archive:                                           
                                                                                                                    
shogitter-engine.json                                                                                               
engine/                                                                                                             
  haitaka_wasm.js                                                                                                   
  haitaka_wasm_bg.wasm                                                                                              
  haitaka_wasm.d.ts                                                                                                 
  haitaka_wasm_bg.wasm.d.ts                                                                                         
  package.json                                                                                                      
  README.md                                                                                                         
  model.nnue                                                                                                        
                                                                                                                    
model.nnue is included only when --nnue is passed.                                                                  
                                                                                                                    
The current haitaka-package.json should be replaced by shogitter-engine.json. It is acceptable to emit both during  
transition, but shogitter-engine.json must be documented as authoritative.                                          
                                                                                                                    
Package command behavior:                                                                                           
                                                                                                                    
- --wasm-dir: defaults to haitaka_wasm/pkg                                                                          
- --output: output .tgz                                                                                             
- --ruleset: defaults to standard, or annan when built with --features annan                                        
- --rule-id: defaults to 0, or 26 when built with --features annan                                                  
- --nnue: optional NNUE file copied to engine/model.nnue                                                            
                                                                                                                    
Before packaging:                                                                                                   
                                                                                                                    
- require haitaka_wasm.js                                                                                           
- require haitaka_wasm_bg.wasm                                                                                      
- copy optional .d.ts, package.json, and README.md if present                                                       
- fail clearly if required files are missing                                                                        
                                                                                                                    
Keep --allow-missing-wasm only as a metadata smoke-test mode. Docs should say archives created with it are not      
loadable by Shogitter.                                                                                              
                                                                                                                    
### Haitaka Manifest Values                                                                                         
                                                                                                                    
For Annan:                                                                                                          
                                                                                                                    
{                                                                                                                   
  "schema": "shogitter-engine-package",                                                                             
  "schemaVersion": 1,                                                                                               
  "engine": {                                                                                                       
    "id": "haitaka-variants",                                                                                       
    "name": "Haitaka Variants",                                                                                     
    "version": "0.1.0",                                                                                             
    "commit": "<git commit or unknown>"                                                                             
  },                                                                                                                
  "runtime": {                                                                                                      
    "kind": "wasm-bindgen",                                                                                         
    "module": "engine/haitaka_wasm.js",                                                                             
    "wasm": "engine/haitaka_wasm_bg.wasm"                                                                           
  },                                                                                                                
  "capabilities": {                                                                                                 
    "protocols": ["shogitter-direct-v1"],                                                                           
    "commands": ["search", "iterative-search", "perft", "dfpn"],                                                    
    "supportsPonder": false,                                                                                        
    "supportsMovetime": true,                                                                                       
    "supportsDepth": true                                                                                           
  },                                                                                                                
  "rules": [                                                                                                        
    {                                                                                                               
      "ruleId": 26,                                                                                                 
      "variant": "annan",                                                                                           
      "positionFormat": "sfen",                                                                                     
      "moveFormat": "usi",                                                                                          
      "startpos": "lnsgkgsnl/1r5b1/p1ppppp1p/1p5p1/9/1P5P1/P1PPPPP1P/1B5R1/LNSGKGSNL b - 1"                         
    }                                                                                                               
  ],                                                                                                                
  "artifacts": {                                                                                                    
    "nnue": null                                                                                                    
  }                                                                                                                 
}                                                                                                                   
                                                                                                                    
For standard shogi:
- ruleId = 0                                                                                                        
- variant = "standard" or "shogi"; choose "shogi" if Shogitter-side naming should align with existing               
  RULE_TO_VARIANT                                                                                                   
- startpos = haitaka::SFEN_STARTPOS                                                                                 
- features = []                                                                                                     
                                                                                                                    
If --nnue is provided:                                                                                              
                                                                                                                    
"artifacts": {                                                                                                      
  "nnue": {                                                                                                         
    "path": "engine/model.nnue",                                                                                    
    "format": "nnue"                                                                                                
  }                                                                                                                 
}                                                                                                                   
                                                                                                                    
### Docs                                                                                                            
                                                                                                                    
Update Haitaka docs:                                                                                                
                                                                                                                    
- root README:                                                                                                      
    - mention Shogitter Engine Package v1 at overview level                                                         
- haitaka_cli/README.md:                                                                                            
    - show wasm-pack build                                                                                          
    - show packaging command                                                                                        
    - show archive layout                                                                                           
    - show manifest example                                                                                         
- docs/shogitter-package.md:                                                                                        
    - define schema fields                                                                                          
    - state that this is the Shogitter-owned first-party contract                                                   
    - distinguish it from legacy Fairy-Stockfish/YaneuraOu emscripten-usi packages                                  
                                                                                                                    
## Test Plan                                                                                                        
                                                                                                                    
### Shogitter Tests                                                                                                 
                                                                                                                    
Add/update tests for:                                                                                               
                                                                                                                    
- manifest tarball extraction:                                                                                      
    - valid wasm-bindgen package succeeds                                                                           
    - missing declared JS module fails                                                                              
    - missing declared WASM fails                                                                                   
    - unsupported schema version fails                                                                              
- legacy tarball extraction:                                                                                        
    - existing Fairy-Stockfish/YaneuraOu fixtures still pass                                                        
- descriptor storage:                                                                                               
    - stores runtimeKind                                                                                            
    - treats missing runtimeKind as emscripten-usi                                                                  
- registration:                                                                                                     
    - manifest package maps rules from manifest.rules                                                               
    - legacy package still maps rules from UCI_Variant and VariantPath                                              
- runtime:                                                                                                          
    - mocked wasm-bindgen adapter returns a best move                                                               
    - legacy adapter still parses bestmove 7g7f ponder 3c3d                                                         
    - user engine worker converts manifest/Haitaka USI best move into a Shogitter KifuCommand                       
                                                                                                                    
Run targeted checks:                                                                                                
                                                                                                                    
pnpm test app/engines app/bot                                                                                       
                                                                                                                    
Run broader checks if feasible:                                                                                     
                                                                                                                    
pnpm test                                                                                                           
pnpm pw:test tests/user-engine-upload.spec.ts                                                                       
                                                                                                                    
### Haitaka Tests                                                                                                   
                                                                                                                    
Add/update tests for:                                                                                               
                                                                                                                    
- manifest serialization                                                                                            
- package command with a fake wasm-pack output directory                                                            
- archive contains shogitter-engine.json                                                                            
- manifest artifact paths exist inside archive                                                                      
- Annan package metadata:                                                                                           
    - runtime.kind = "wasm-bindgen"                                                                                 
    - ruleId = 26                                                                                                   
    - variant = "annan"                                                                                             
    - positionFormat = "sfen"                                                                                       
    - moveFormat = "usi"                                                                                            
                                                                                                                    
Run:                                                                                                                
                                                                                                                    
cargo test --workspace                                                                                              
cargo test --workspace --features annan                                                                             
                                                                                                                    
Manual package verification:                                                                                        
                                                                                                                    
wasm-pack build haitaka_wasm --target web --out-dir pkg --release --features annan                                  
                                                                                                                    
cargo run -p haitaka_cli --release --features annan -- package \                                                    
  --wasm-dir haitaka_wasm/pkg \                                                                                     
  --ruleset annan \                                                                                                 
  --rule-id 26 \                                                                                                    
  --output target/haitaka-variants-annan.tgz                                                                        
                                                                                                                    
Then inspect:                                                                                                       
                                                                                                                    
tar -tzf target/haitaka-variants-annan.tgz                                                                          
tar -xOzf target/haitaka-variants-annan.tgz shogitter-engine.json
## Acceptance Criteria                                                                                              
                                                                                                                    
I will consider the work complete when all of these are true:                                                       
                                                                                                                    
- Haitaka creates a valid Shogitter Engine Package v1 .tgz.                                                         
- The Haitaka archive contains:                                                                                     
    - shogitter-engine.json                                                                                         
    - engine/haitaka_wasm.js                                                                                        
    - engine/haitaka_wasm_bg.wasm                                                                                   
- The Haitaka manifest declares:                                                                                    
    - schema = "shogitter-engine-package"                                                                           
    - schemaVersion = 1                                                                                             
    - runtime.kind = "wasm-bindgen"                                                                                 
    - rule 26 / annan                                                                                               
    - positionFormat = "sfen"                                                                                       
    - moveFormat = "usi"                                                                                            
- Shogitter can upload/register the Haitaka .tgz from /my/engines.                                                  
- Shogitter displays rule 26 support based on the manifest, without requiring UCI_Variant.                          
- Shogitter can start a rule 26 game against the uploaded Haitaka engine.                                           
- Haitaka returns a legal USI move and Shogitter converts it into a valid KifuCommand.                              
- Existing Fairy-Stockfish and YaneuraOu upload support still works.                                                
- Existing legacy emscripten-usi probe behavior is not regressed.                                                   
- Docs in both repos clearly distinguish:                                                                           
    - Shogitter Engine Package v1 first-party manifest packages                                                     
    - legacy Fairy-Stockfish/YaneuraOu Emscripten USI packages                                                      
    - Haitaka’s wasm-bindgen runtime path