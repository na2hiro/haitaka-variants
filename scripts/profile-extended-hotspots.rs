use haitaka::{extended::compiled as api,hot_probe};
use std::{alloc::{GlobalAlloc,Layout,System},cell::Cell,hint::black_box,time::{Duration,Instant}};
struct Counting;
thread_local! {
    static ENABLED:Cell<bool>=const{Cell::new(false)};
    static ALLOCATIONS:Cell<(u64,u64)>=const{Cell::new((0,0))};
}
fn count(bytes:usize) {
    let _=ENABLED.try_with(|enabled| if enabled.get() {
        ALLOCATIONS.with(|s|{let(n,b)=s.get();s.set((n+1,b+bytes as u64));});
    });
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self,l:Layout)->*mut u8 {count(l.size());unsafe{System.alloc(l)}}
    unsafe fn dealloc(&self,p:*mut u8,l:Layout){unsafe{System.dealloc(p,l)}}
    unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{count(n);unsafe{System.realloc(p,l,n)}}
    unsafe fn alloc_zeroed(&self,l:Layout)->*mut u8{count(l.size());unsafe{System.alloc_zeroed(l)}}
}
#[global_allocator] static ALLOCATOR:Counting=Counting;
fn main(){
 let args:Vec<String>=std::env::args().collect();let depth:u8=args[2].parse().unwrap();
 let mut game=api::CompiledGame::parse(&api::start_position_json()).unwrap();
 let nodes=black_box(game.perft(depth));let mut samples=vec![];
 for _ in 0..3 {
  hot_probe::reset();let start=Instant::now();let mut repetitions=0u64;
  while start.elapsed()<Duration::from_secs(3){assert_eq!(black_box(&mut game).perft(black_box(depth)),nodes);repetitions+=1;}
  let seconds=start.elapsed().as_secs_f64();let(calls,nanos)=hot_probe::stats();
  samples.push(serde_json::json!({"seconds":seconds,"repetitions":repetitions,"calls":calls,"nanos":nanos}));
 }
 ALLOCATIONS.with(|s|s.set((0,0)));ENABLED.with(|s|s.set(true));
 assert_eq!(black_box(&mut game).perft(depth),nodes);
 ENABLED.with(|s|s.set(false));let(allocations,requested_bytes)=ALLOCATIONS.with(Cell::get);
 println!("{}",serde_json::json!({"rule":args[1],"depth":depth,"nodesPerPerft":nodes,"samples":samples,"allocationsPerPerft":allocations,"requestedAllocationBytesPerPerft":requested_bytes}));
}
