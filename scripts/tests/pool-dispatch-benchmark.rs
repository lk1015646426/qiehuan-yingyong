 // Isolated HTTP benchmark for local dispatch; never contacts a provider.
#[path = "../../src-tauri/src/modules/pool_dispatch.rs"]
mod pool_dispatch;
use std::sync::{Arc, Barrier, atomic::{AtomicBool,AtomicUsize,Ordering}};
use std::thread;
use std::time::{Duration,Instant};
use tiny_http::{Server,Response};

fn main() {
    let stub_latency = Duration::from_millis(100);
    for concurrency in [1usize, 4, 8, 16, 32] {
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let endpoint = format!("http://{}/mock",server.server_addr());
        let stopping = Arc::new(AtomicBool::new(false));
        let active=Arc::new(AtomicUsize::new(0));
        let peak=Arc::new(AtomicUsize::new(0));
        let (s,stop,a,p)=(server.clone(),stopping.clone(),active.clone(),peak.clone());
        let listener=thread::spawn(move||pool_dispatch::dispatch(&s,&stop,|request|{
            let now=a.fetch_add(1,Ordering::SeqCst)+1;
            p.fetch_max(now,Ordering::SeqCst);
            thread::sleep(stub_latency);
            a.fetch_sub(1,Ordering::SeqCst);
            let _=request.respond(Response::from_string("mock"));
        }));
        let gate=Arc::new(Barrier::new(concurrency+1));
        let client=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build().unwrap();
        let mut callers=Vec::new();
        for _ in 0..concurrency {
            let (c,u,g)=(client.clone(),endpoint.clone(),gate.clone());
            callers.push(thread::spawn(move||{
                g.wait();
                let mut rows=Vec::new();
                for _ in 0..8 {
                    let begin=Instant::now();
                    let status=c.get(&u).send().map(|r|{let status=r.status().as_u16();let _=r.bytes();status}).unwrap_or(0);
                    rows.push((status,begin.elapsed().as_millis()));
                }
                rows
            }));
        }
        let begin=Instant::now();
        gate.wait();
        let mut results=Vec::new();
        for t in callers {results.extend(t.join().unwrap());}
        let total=begin.elapsed();
        stopping.store(true,Ordering::SeqCst);
        server.unblock();
        listener.join().unwrap();
        let ok=results.iter().filter(|r|r.0==200).count();
        let busy=results.iter().filter(|r|r.0==429).count();
        let mut times:Vec<u128>=results.iter().filter(|r|r.0==200).map(|r|r.1).collect();
        times.sort_unstable();
        let p95=if times.is_empty(){0}else{times[(times.len()*95).div_ceil(100)-1]};
        println!("clients={} requests={} ok={} busy={} other={} peak={} elapsed_ms={} successful_rps={:.2} successful_p95_ms={}",concurrency,results.len(),ok,busy,results.len()-ok-busy,peak.load(Ordering::SeqCst),total.as_millis(),ok as f64/total.as_secs_f64(),p95);
        assert!(peak.load(Ordering::SeqCst)<=pool_dispatch::MAX_ACTIVE);
        assert_eq!(results.len(),concurrency*8);
        assert_eq!(results.len(),ok+busy);
    }
}